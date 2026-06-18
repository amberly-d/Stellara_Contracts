use soroban_sdk::{
    contracttype, symbol_short, Address, Env, Symbol, Vec, Map, Bytes, BytesN,
    contracterror, require_auth
};
use shared::governance::{GovernanceManager, GovernanceRole};
use shared::events::{
    extended_topics,
    DidCreatedEvent, DidUpdatedEvent, DidDeactivatedEvent,
    VerificationMethodAddedEvent, ServiceAddedEvent,
};

// DID Document structure
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DIDDocument {
    pub id: Symbol,
    pub verification_methods: Vec<VerificationMethod>,
    pub authentication: Vec<Symbol>,
    pub assertion_method: Vec<Symbol>,
    pub key_agreement: Vec<Symbol>,
    pub service: Vec<Service>,
    pub created_at: u64,
    pub updated_at: u64,
    pub deactivated: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationMethod {
    pub id: Symbol,
    pub type_: Symbol,  // "Ed25519VerificationKey2018", etc.
    pub controller: Symbol,
    pub public_key: Bytes, // Multibase encoded public key
    pub created_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Service {
    pub id: Symbol,
    pub type_: Symbol,
    pub service_endpoint: Symbol,
    pub created_at: u64,
}

// DID Method types
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum DIDMethod {
    Stellar = 0,
    Key = 1,
}

// Error codes
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum DIDRegistryError {
    InvalidDIDFormat = 3001,
    DIDNotFound = 3002,
    Unauthorized = 3003,
    InvalidVerificationMethod = 3004,
    DuplicateService = 3005,
    AlreadyDeactivated = 3006,
    InvalidPublicKey = 3007,
    GovernanceError = 3008,
}

pub struct DIDRegistryContract;

#[soroban_sdk::contractimpl]
impl DIDRegistryContract {
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
        
        // Initialize DID counter
        let counter_key = symbol_short!("did_cnt");
        env.storage().persistent().set(&counter_key, &0u64);
    }

    // Create DID document for did:stellar
    pub fn create_stellar_did(
        env: Env,
        stellar_address: Address,
        verification_methods: Vec<VerificationMethod>,
        services: Vec<Service>,
    ) -> Symbol {
        // Validate stellar address format
        let did_str = format!("did:stellar:{}", stellar_address);
        let did_id = symbol_short!(&did_str);

        // Check if DID already exists
        if Self::get_did_document(env.clone(), did_id.clone()).is_ok() {
            panic!("DID already exists");
        }

        // Create authentication list from verification methods
        let mut authentication = Vec::new(&env);
        for vm in verification_methods.iter() {
            authentication.push_back(vm.id.clone());
        }

        let document = DIDDocument {
            id: did_id.clone(),
            verification_methods,
            authentication,
            assertion_method: Vec::new(&env),
            key_agreement: Vec::new(&env),
            service: services,
            created_at: env.ledger().timestamp(),
            updated_at: env.ledger().timestamp(),
            deactivated: false,
        };

        // Store DID document
        let dids_key = symbol_short!("dids");
        let mut dids: Map<Symbol, DIDDocument> = env
            .storage()
            .persistent()
            .get(&dids_key)
            .unwrap_or_else(|| Map::new(&env));

        dids.set(did_id.clone(), document);
        env.storage().persistent().set(&dids_key, &dids);

        // Update counter
        let counter_key = symbol_short!("did_cnt");
        let count: u64 = env.storage().persistent().get(&counter_key).unwrap_or(0);
        env.storage().persistent().set(&counter_key, &(count + 1));

        env.events().publish(
            (extended_topics::DID_CREATED,),
            DidCreatedEvent {
                did: did_id.clone(),
                controller: stellar_address,
                method: symbol_short!("stellar"),
                timestamp: env.ledger().timestamp(),
            },
        );

        did_id
    }

    // Create DID document for did:key
    pub fn create_key_did(
        env: Env,
        public_key: Bytes, // Multibase encoded
        verification_methods: Vec<VerificationMethod>,
        services: Vec<Service>,
    ) -> Symbol {
        // Validate public key format
        if public_key.is_empty() {
            panic!("Invalid public key");
        }

        // Generate did:key format
        let did_str = format!("did:key:{}", String::from_utf8_lossy(&public_key));
        let did_id = symbol_short!(&did_str);

        // Check if DID already exists
        if Self::get_did_document(env.clone(), did_id.clone()).is_ok() {
            panic!("DID already exists");
        }

        // Create authentication list
        let mut authentication = Vec::new(&env);
        for vm in verification_methods.iter() {
            authentication.push_back(vm.id.clone());
        }

        let document = DIDDocument {
            id: did_id.clone(),
            verification_methods,
            authentication,
            assertion_method: Vec::new(&env),
            key_agreement: Vec::new(&env),
            service: services,
            created_at: env.ledger().timestamp(),
            updated_at: env.ledger().timestamp(),
            deactivated: false,
        };

        // Store DID document
        let dids_key = symbol_short!("dids");
        let mut dids: Map<Symbol, DIDDocument> = env
            .storage()
            .persistent()
            .get(&dids_key)
            .unwrap_or_else(|| Map::new(&env));

        dids.set(did_id.clone(), document);
        env.storage().persistent().set(&dids_key, &dids);

        // Update counter
        let counter_key = symbol_short!("did_cnt");
        let count: u64 = env.storage().persistent().get(&counter_key).unwrap_or(0);
        env.storage().persistent().set(&counter_key, &(count + 1));

        env.events().publish(
            (extended_topics::DID_CREATED,),
            DidCreatedEvent {
                did: did_id.clone(),
                controller: env.current_contract_address(),
                method: symbol_short!("key"),
                timestamp: env.ledger().timestamp(),
            },
        );

        did_id
    }

    // Resolve DID document
    pub fn resolve_did(env: Env, did: Symbol) -> DIDDocument {
        Self::get_did_document(env, did).unwrap()
    }

    // Update DID document (requires authentication)
    pub fn update_did_document(
        env: Env,
        did: Symbol,
        verification_methods: Option<Vec<VerificationMethod>>,
        services: Option<Vec<Service>>,
    ) {
        let caller = env.current_contract_address();
        
        // Get existing document
        let mut document = Self::get_did_document(env.clone(), did.clone()).unwrap();
        
        // Check if caller is authorized (simplified - in production, use proper DID auth)
        require_auth!(&caller);

        if document.deactivated {
            panic!("DID is deactivated");
        }

        // Update verification methods if provided
        if let Some(new_vms) = verification_methods {
            document.verification_methods = new_vms;
            
            // Update authentication list
            document.authentication = Vec::new(&env);
            for vm in document.verification_methods.iter() {
                document.authentication.push_back(vm.id.clone());
            }
        }

        // Update services if provided
        if let Some(new_services) = services {
            document.service = new_services;
        }

        document.updated_at = env.ledger().timestamp();

        // Store updated document
        let dids_key = symbol_short!("dids");
        let mut dids: Map<Symbol, DIDDocument> = env
            .storage()
            .persistent()
            .get(&dids_key)
            .unwrap_or_else(|| Map::new(&env));

        dids.set(did.clone(), document);
        env.storage().persistent().set(&dids_key, &dids);

        env.events().publish(
            (extended_topics::DID_UPDATED,),
            DidUpdatedEvent {
                did,
                controller: env.current_contract_address(),
                timestamp: env.ledger().timestamp(),
            },
        );
    }

    // Deactivate DID
    pub fn deactivate_did(env: Env, did: Symbol) {
        let caller = env.current_contract_address();
        
        // Get existing document
        let mut document = Self::get_did_document(env.clone(), did.clone()).unwrap();
        
        // Check authorization
        require_auth!(&caller);

        if document.deactivated {
            panic!("DID already deactivated");
        }

        document.deactivated = true;
        document.updated_at = env.ledger().timestamp();

        // Store updated document
        let dids_key = symbol_short!("dids");
        let mut dids: Map<Symbol, DIDDocument> = env
            .storage()
            .persistent()
            .get(&dids_key)
            .unwrap_or_else(|| Map::new(&env));

        dids.set(did.clone(), document);
        env.storage().persistent().set(&dids_key, &dids);

        env.events().publish(
            (extended_topics::DID_DEACTIVATED,),
            DidDeactivatedEvent {
                did,
                deactivated_by: env.current_contract_address(),
                timestamp: env.ledger().timestamp(),
            },
        );
    }

    // Add verification method
    pub fn add_verification_method(
        env: Env,
        did: Symbol,
        verification_method: VerificationMethod,
    ) {
        let caller = env.current_contract_address();
        
        // Get existing document
        let mut document = Self::get_did_document(env.clone(), did.clone()).unwrap();
        
        // Check authorization
        require_auth!(&caller);

        if document.deactivated {
            panic!("DID is deactivated");
        }

        // Check for duplicate
        for existing_vm in document.verification_methods.iter() {
            if existing_vm.id == verification_method.id {
                panic!("Verification method already exists");
            }
        }

        document.verification_methods.push_back(verification_method.clone());
        document.authentication.push_back(verification_method.id);
        document.updated_at = env.ledger().timestamp();

        // Store updated document
        let dids_key = symbol_short!("dids");
        let mut dids: Map<Symbol, DIDDocument> = env
            .storage()
            .persistent()
            .get(&dids_key)
            .unwrap_or_else(|| Map::new(&env));

        dids.set(did.clone(), document);
        env.storage().persistent().set(&dids_key, &dids);

        env.events().publish(
            (extended_topics::VERIF_METHOD_ADDED,),
            VerificationMethodAddedEvent {
                did: did.clone(),
                method_id: verification_method.id.clone(),
                controller: env.current_contract_address(),
                timestamp: env.ledger().timestamp(),
            },
        );
    }

    // Add service
    pub fn add_service(env: Env, did: Symbol, service: Service) {
        let caller = env.current_contract_address();
        
        // Get existing document
        let mut document = Self::get_did_document(env.clone(), did.clone()).unwrap();
        
        // Check authorization
        require_auth!(&caller);

        if document.deactivated {
            panic!("DID is deactivated");
        }

        // Check for duplicate
        for existing_service in document.service.iter() {
            if existing_service.id == service.id {
                panic!("Service already exists");
            }
        }

        document.service.push_back(service.clone());
        document.updated_at = env.ledger().timestamp();

        // Store updated document
        let dids_key = symbol_short!("dids");
        let mut dids: Map<Symbol, DIDDocument> = env
            .storage()
            .persistent()
            .get(&dids_key)
            .unwrap_or_else(|| Map::new(&env));

        dids.set(did.clone(), document);
        env.storage().persistent().set(&dids_key, &dids);

        env.events().publish(
            (extended_topics::SERVICE_ADDED,),
            ServiceAddedEvent {
                did,
                service_id: service.id,
                controller: env.current_contract_address(),
                timestamp: env.ledger().timestamp(),
            },
        );
    }

    // Get DID document (internal helper)
    fn get_did_document(env: Env, did: Symbol) -> Result<DIDDocument, DIDRegistryError> {
        let dids_key = symbol_short!("dids");
        let dids: Map<Symbol, DIDDocument> = env
            .storage()
            .persistent()
            .get(&dids_key)
            .ok_or(DIDRegistryError::DIDNotFound)?;

        dids.get(did).ok_or(DIDRegistryError::DIDNotFound)
    }

    // Get all DIDs (for admin)
    pub fn get_all_dids(env: Env) -> Vec<Symbol> {
        let dids_key = symbol_short!("dids");
        let dids: Map<Symbol, DIDDocument> = env
            .storage()
            .persistent()
            .get(&dids_key)
            .unwrap_or_else(|| Map::new(&env));

        let mut result = Vec::new(&env);
        for (did_id, _) in dids.iter() {
            result.push_back(did_id);
        }
        result
    }

    // Get DID count
    pub fn get_did_count(env: Env) -> u64 {
        let counter_key = symbol_short!("did_cnt");
        env.storage().persistent().get(&counter_key).unwrap_or(0)
    }
}