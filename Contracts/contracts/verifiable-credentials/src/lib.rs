use soroban_sdk::{
    contracttype, symbol_short, Address, Env, Symbol, Vec, Map, Bytes, BytesN,
    contracterror, require_auth
};
use shared::governance::{GovernanceManager, GovernanceRole};
use shared::events::{extended_topics, CredentialIssuedEvent, CredentialRevokedEvent};

// Verifiable Credential structure
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiableCredential {
    pub id: Symbol,
    pub context: Symbol,
    pub type_: Vec<Symbol>,
    pub issuer: Symbol,  // DID of issuer
    pub issuance_date: u64,
    pub expiration_date: Option<u64>,
    pub credential_subject: CredentialSubject,
    pub proof: Proof,
    pub credential_status: Option<CredentialStatus>,
    pub created_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CredentialSubject {
    pub id: Symbol,  // DID of subject
    pub claims: Map<Symbol, Symbol>,  // Key-value pairs of claims
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Proof {
    pub type_: Symbol,
    pub created: u64,
    pub verification_method: Symbol,
    pub proof_purpose: Symbol,
    pub proof_value: Bytes,  // Base64 encoded signature
    pub domain: Option<Symbol>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CredentialStatus {
    pub id: Symbol,
    pub type_: Symbol,
    pub status: Symbol,  // "valid", "revoked", "suspended"
    pub revocation_reason: Option<Symbol>,
}

// Credential types
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum CredentialType {
    KYCVerified = 0,
    AccreditedInvestor = 1,
    EducationalAchievement = 2,
    ProfessionalLicense = 3,
    Custom = 4,
}

// Revocation registry entry
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevocationEntry {
    pub credential_id: Symbol,
    pub revoker: Symbol,  // DID of revoker
    pub revocation_date: u64,
    pub reason: Symbol,
    pub proof: Bytes,
}

// Error codes
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum VCError {
    InvalidCredential = 4001,
    UnauthorizedIssuer = 4002,
    CredentialNotFound = 4003,
    AlreadyRevoked = 4004,
    ExpiredCredential = 4005,
    InvalidProof = 4006,
    InvalidSubject = 4007,
    GovernanceError = 4008,
}

pub struct VerifiableCredentialsContract;

#[soroban_sdk::contractimpl]
impl VerifiableCredentialsContract {
    // Initialize contract with governance
    pub fn initialize(
        env: Env,
        admin: Address,
        approvers: Vec<Address>,
        executor: Address,
    ) {
        // Set up governance roles
        let roles_key = symbol_short!("roles");
        let mut role_map: Map<Address, GovernanceRole> = Map::new(&env);
        
        role_map.set(admin.clone(), GovernanceRole::Admin);
        for approver in approvers.iter() {
            role_map.set(approver.clone(), GovernanceRole::Approver);
        }
        role_map.set(executor, GovernanceRole::Executor);
        
        env.storage().persistent().set(&roles_key, &role_map);
        
        // Initialize credential counter
        let counter_key = symbol_short!("vc_cnt");
        env.storage().persistent().set(&counter_key, &0u64);

        // Initialize revocation registry
        let revocation_key = symbol_short!("revocatn");
        let revocations: Map<Symbol, RevocationEntry> = Map::new(&env);
        env.storage().persistent().set(&revocation_key, &revocations);
    }

    // Issue a verifiable credential
    pub fn issue_credential(
        env: Env,
        issuer_did: Symbol,
        subject_did: Symbol,
        credential_type: CredentialType,
        claims: Map<Symbol, Symbol>,
        expiration_date: Option<u64>,
        proof: Proof,
    ) -> Result<Symbol, VCError> {
        // Verify issuer is authorized (simplified - in production, check issuer registry)
        let caller = env.current_contract_address();
        require_auth!(&caller);

        // Validate inputs
        if proof.proof_value.is_empty() {
            return Err(VCError::InvalidProof);
        }
        if let Some(exp) = expiration_date {
            if exp <= env.ledger().timestamp() {
                return Err(VCError::InvalidCredential);
            }
        }

        // Generate credential ID
        let counter_key = symbol_short!("vc_cnt");
        let count: u64 = env.storage().persistent().get(&counter_key).unwrap_or(0);
        let credential_id = symbol_short!(&format!("vc-{}", count + 1));

        // Create credential type vector
        let mut type_vec = Vec::new(&env);
        type_vec.push_back(symbol_short!("vc"));
        
        match credential_type {
            CredentialType::KYCVerified => type_vec.push_back(symbol_short!("kyc_vc")),
            CredentialType::AccreditedInvestor => type_vec.push_back(symbol_short!("acc_vc")),
            CredentialType::EducationalAchievement => type_vec.push_back(symbol_short!("edu_vc")),
            CredentialType::ProfessionalLicense => type_vec.push_back(symbol_short!("pro_vc")),
            CredentialType::Custom => type_vec.push_back(symbol_short!("cust_vc")),
        }

        // Create credential subject
        let subject = CredentialSubject {
            id: subject_did,
            claims,
        };

        // Create credential status
        let status = CredentialStatus {
            id: symbol_short!(&format!("status-{}", count + 1)),
            type_: symbol_short!("csl2021"),
            status: symbol_short!("valid"),
            revocation_reason: None,
        };

        // Create verifiable credential
        let credential = VerifiableCredential {
            id: credential_id.clone(),
            context: symbol_short!("w3c_ctx"),
            type_: type_vec,
            issuer: issuer_did,
            issuance_date: env.ledger().timestamp(),
            expiration_date,
            credential_subject: subject,
            proof,
            credential_status: Some(status),
            created_at: env.ledger().timestamp(),
        };

        // Store credential
        let credentials_key = symbol_short!("creds");
        let mut credentials: Map<Symbol, VerifiableCredential> = env
            .storage()
            .persistent()
            .get(&credentials_key)
            .unwrap_or_else(|| Map::new(&env));

        credentials.set(credential_id.clone(), credential);
        env.storage().persistent().set(&credentials_key, &credentials);

        // Update counter
        env.storage().persistent().set(&counter_key, &(count + 1));

        env.events().publish(
            (extended_topics::CREDENTIAL_ISSUED,),
            CredentialIssuedEvent {
                credential_id: credential_id.clone(),
                issuer_did,
                subject_did: credential.credential_subject.id.clone(),
                credential_type: credential.type_.get(1).unwrap_or(symbol_short!("Custom")),
                timestamp: env.ledger().timestamp(),
            },
        );

        credential_id
        Ok(credential_id)
    }

    // Verify a verifiable credential
    pub fn verify_credential(env: Env, credential_id: Symbol) -> Result<bool, VCError> {
        // Get credential
        let credential = Self::get_credential(env.clone(), credential_id.clone())?;

        // Check if credential is revoked
        if Self::is_revoked(env.clone(), credential_id.clone()) {
            return Ok(false);
        }

        // Check expiration
        if let Some(expiration) = credential.expiration_date {
            if env.ledger().timestamp() > expiration {
                return Ok(false);
            }
        }

        // Verify proof (simplified - in production, implement proper cryptographic verification)
        if credential.proof.proof_value.is_empty() {
            return Err(VCError::InvalidProof);
        }

        Ok(true)
    }

    // Revoke a verifiable credential
    pub fn revoke_credential(
        env: Env,
        credential_id: Symbol,
        revoker_did: Symbol,
        reason: Symbol,
        proof: Bytes,
    ) {
        // Get credential
        let mut credential = Self::get_credential(env.clone(), credential_id.clone()).unwrap();

        // Check authorization (issuer or authorized revoker)
        let caller = env.current_contract_address();
        require_auth!(&caller);

        // Check if already revoked
        if Self::is_revoked(env.clone(), credential_id.clone()) {
            panic!("Credential already revoked");
        }

        // Create revocation entry
        let revocation = RevocationEntry {
            credential_id: credential_id.clone(),
            revoker: revoker_did,
            revocation_date: env.ledger().timestamp(),
            reason: reason.clone(),
            proof,
        };

        // Store revocation
        let revocations_key = symbol_short!("revocatn");
        let mut revocations: Map<Symbol, RevocationEntry> = env
            .storage()
            .persistent()
            .get(&revocations_key)
            .unwrap_or_else(|| Map::new(&env));

        revocations.set(credential_id.clone(), revocation);
        env.storage().persistent().set(&revocations_key, &revocations);

        // Update credential status
        if let Some(mut status) = credential.credential_status {
            status.status = symbol_short!("revoked");
            status.revocation_reason = Some(reason);
            credential.credential_status = Some(status);
        }

        // Store updated credential
        let credentials_key = symbol_short!("creds");
        let mut credentials: Map<Symbol, VerifiableCredential> = env
            .storage()
            .persistent()
            .get(&credentials_key)
            .unwrap_or_else(|| Map::new(&env));

        credentials.set(credential_id.clone(), credential);
        env.storage().persistent().set(&credentials_key, &credentials);

        env.events().publish(
            (extended_topics::CREDENTIAL_REVOKED,),
            CredentialRevokedEvent {
                credential_id,
                revoked_by: env.current_contract_address(),
                reason,
                timestamp: env.ledger().timestamp(),
            },
        );
    }

    // Get credential details
    pub fn get_credential_details(env: Env, credential_id: Symbol) -> VerifiableCredential {
        Self::get_credential(env, credential_id).unwrap()
    }

    // Get credentials by subject
    pub fn get_credentials_by_subject(env: Env, subject_did: Symbol) -> Vec<Symbol> {
        let credentials_key = symbol_short!("creds");
        let credentials: Map<Symbol, VerifiableCredential> = env
            .storage()
            .persistent()
            .get(&credentials_key)
            .unwrap_or_else(|| Map::new(&env));

        let mut result = Vec::new(&env);
        for (cred_id, credential) in credentials.iter() {
            if credential.credential_subject.id == subject_did {
                result.push_back(cred_id);
            }
        }
        result
    }

    // Get credentials by issuer
    pub fn get_credentials_by_issuer(env: Env, issuer_did: Symbol) -> Vec<Symbol> {
        let credentials_key = symbol_short!("creds");
        let credentials: Map<Symbol, VerifiableCredential> = env
            .storage()
            .persistent()
            .get(&credentials_key)
            .unwrap_or_else(|| Map::new(&env));

        let mut result = Vec::new(&env);
        for (cred_id, credential) in credentials.iter() {
            if credential.issuer == issuer_did {
                result.push_back(cred_id);
            }
        }
        result
    }

    // Get revocation status
    pub fn get_revocation_status(env: Env, credential_id: Symbol) -> Option<RevocationEntry> {
        let revocations_key = symbol_short!("revocatn");
        let revocations: Map<Symbol, RevocationEntry> = env
            .storage()
            .persistent()
            .get(&revocations_key)
            .unwrap_or_else(|| Map::new(&env));

        revocations.get(credential_id)
    }

    // Get credential count
    pub fn get_credential_count(env: Env) -> u64 {
        let counter_key = symbol_short!("vc_cnt");
        env.storage().persistent().get(&counter_key).unwrap_or(0)
    }

    // Get all credentials (for admin)
    pub fn get_all_credentials(env: Env) -> Vec<Symbol> {
        let credentials_key = symbol_short!("creds");
        let credentials: Map<Symbol, VerifiableCredential> = env
            .storage()
            .persistent()
            .get(&credentials_key)
            .unwrap_or_else(|| Map::new(&env));

        let mut result = Vec::new(&env);
        for (cred_id, _) in credentials.iter() {
            result.push_back(cred_id);
        }
        result
    }

    // Check if credential is revoked (internal helper)
    fn is_revoked(env: Env, credential_id: Symbol) -> bool {
        let revocations_key = symbol_short!("revocatn");
        let revocations: Map<Symbol, RevocationEntry> = env
            .storage()
            .persistent()
            .get(&revocations_key)
            .unwrap_or_else(|| Map::new(&env));

        revocations.contains_key(credential_id)
    }

    // Get credential (internal helper)
    fn get_credential(env: Env, credential_id: Symbol) -> Result<VerifiableCredential, VCError> {
        let credentials_key = symbol_short!("creds");
        let credentials: Map<Symbol, VerifiableCredential> = env
            .storage()
            .persistent()
            .get(&credentials_key)
            .ok_or(VCError::CredentialNotFound)?;

        credentials.get(credential_id).ok_or(VCError::CredentialNotFound)
    }
}