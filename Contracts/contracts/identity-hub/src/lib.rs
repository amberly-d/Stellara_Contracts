use soroban_sdk::{
    contracttype, symbol_short, Address, Env, Symbol, Vec, Map, Bytes, BytesN,
    contracterror, require_auth
};
use shared::governance::{GovernanceManager, GovernanceRole};
use shared::events::{extended_topics, HubCreatedEvent, DataEntryAddedEvent, PermissionGrantedEvent, PermissionRevokedEvent, SelectiveDisclosureCreatedEvent};

// Identity Hub data structure
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdentityHub {
    pub id: Symbol,
    pub owner_did: Symbol,
    pub data_entries: Vec<DataEntry>,
    pub permissions: Vec<Permission>,
    pub created_at: u64,
    pub updated_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataEntry {
    pub id: Symbol,
    pub type_: Symbol,  // "credential", "profile", "document", etc.
    pub encrypted_data: Bytes,  // Encrypted data payload
    pub hash: Bytes,  // Hash of unencrypted data for integrity
    pub created_at: u64,
    pub expires_at: Option<u64>,
    pub metadata: Map<Symbol, Symbol>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Permission {
    pub id: Symbol,
    pub granter_did: Symbol,  // DID granting permission
    pub grantee_did: Symbol,  // DID receiving permission
    pub data_entry_id: Symbol,  // Specific data entry this applies to
    pub permission_type: PermissionType,
    pub conditions: Vec<Condition>,
    pub created_at: u64,
    pub expires_at: Option<u64>,
    pub active: bool,
}

#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum PermissionType {
    Read = 0,
    Write = 1,
    Share = 2,
    Verify = 3,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Condition {
    pub type_: Symbol,  // "time_limit", "purpose", "revocation", etc.
    pub value: Symbol,
    pub operator: Symbol,  // "equals", "greater_than", "contains", etc.
}

// Selective disclosure structure
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectiveDisclosure {
    pub id: Symbol,
    pub presenter_did: Symbol,
    pub verifier_did: Symbol,
    pub data_entry_id: Symbol,
    pub disclosed_fields: Vec<Symbol>,  // Which fields to disclose
    pub proof: Bytes,  // ZKP or signature proof
    pub nonce: Bytes,
    pub created_at: u64,
    pub expires_at: u64,
}

// Error codes
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum IdentityHubError {
    HubNotFound = 5001,
    Unauthorized = 5002,
    DataEntryNotFound = 5003,
    PermissionDenied = 5004,
    InvalidEncryption = 5005,
    ExpiredData = 5006,
    InvalidPermission = 5007,
    GovernanceError = 5008,
}

pub struct IdentityHubContract;

#[soroban_sdk::contractimpl]
impl IdentityHubContract {
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
        
        // Initialize hub counter
        let counter_key = symbol_short!("hub_cnt");
        env.storage().persistent().set(&counter_key, &0u64);
    }

    // Create identity hub for a DID
    pub fn create_hub(env: Env, owner_did: Symbol) -> Symbol {
        // Check authorization (simplified - in production, verify DID ownership)
        let caller = env.current_contract_address();
        require_auth!(&caller);

        // Generate hub ID
        let counter_key = symbol_short!("hub_cnt");
        let count: u64 = env.storage().persistent().get(&counter_key).unwrap_or(0);
        let hub_id = symbol_short!(&format!("hub-{}", count + 1));

        // Check if hub already exists for this DID
        if Self::get_hub_by_owner(env.clone(), owner_did.clone()).is_ok() {
            panic!("Hub already exists for this DID");
        }

        let hub = IdentityHub {
            id: hub_id.clone(),
            owner_did: owner_did.clone(),
            data_entries: Vec::new(&env),
            permissions: Vec::new(&env),
            created_at: env.ledger().timestamp(),
            updated_at: env.ledger().timestamp(),
        };

        // Store hub
        let hubs_key = symbol_short!("hubs");
        let mut hubs: Map<Symbol, IdentityHub> = env
            .storage()
            .persistent()
            .get(&hubs_key)
            .unwrap_or_else(|| Map::new(&env));

        hubs.set(hub_id.clone(), hub);
        
        // Store owner to hub mapping
        let owner_map_key = symbol_short!("own_hub");
        let mut owner_map: Map<Symbol, Symbol> = env
            .storage()
            .persistent()
            .get(&owner_map_key)
            .unwrap_or_else(|| Map::new(&env));

        owner_map.set(owner_did, hub_id.clone());
        
        // Update storage
        env.storage().persistent().set(&hubs_key, &hubs);
        env.storage().persistent().set(&owner_map_key, &owner_map);
        env.storage().persistent().set(&counter_key, &(count + 1));

        env.events().publish(
            (extended_topics::HUB_CREATED,),
            HubCreatedEvent {
                hub_id: hub_id.clone(),
                owner_did,
                timestamp: env.ledger().timestamp(),
            },
        );

        hub_id
    }

    // Add data entry to hub
    pub fn add_data_entry(
        env: Env,
        hub_id: Symbol,
        data_type: Symbol,
        encrypted_data: Bytes,
        hash: Bytes,
        expires_at: Option<u64>,
        metadata: Map<Symbol, Symbol>,
    ) -> Symbol {
        // Get hub
        let mut hub = Self::get_hub(env.clone(), hub_id.clone()).unwrap();
        
        // Check authorization (owner only)
        let caller = env.current_contract_address();
        require_auth!(&caller);

        // Generate data entry ID
        let entry_id = symbol_short!(&format!("data-{}", hub.data_entries.len() + 1));

        let data_entry = DataEntry {
            id: entry_id.clone(),
            type_: data_type,
            encrypted_data,
            hash,
            created_at: env.ledger().timestamp(),
            expires_at,
            metadata,
        };

        hub.data_entries.push_back(data_entry);
        hub.updated_at = env.ledger().timestamp();

        // Store updated hub
        let hubs_key = symbol_short!("hubs");
        let mut hubs: Map<Symbol, IdentityHub> = env
            .storage()
            .persistent()
            .get(&hubs_key)
            .unwrap_or_else(|| Map::new(&env));

        hubs.set(hub_id.clone(), hub);
        env.storage().persistent().set(&hubs_key, &hubs);

        env.events().publish(
            (extended_topics::DATA_ENTRY_ADDED,),
            DataEntryAddedEvent {
                hub_id,
                entry_id: entry_id.clone(),
                added_by: env.current_contract_address(),
                timestamp: env.ledger().timestamp(),
            },
        );

        entry_id
    }

    // Grant permission to access data
    pub fn grant_permission(
        env: Env,
        hub_id: Symbol,
        grantee_did: Symbol,
        data_entry_id: Symbol,
        permission_type: PermissionType,
        conditions: Vec<Condition>,
        expires_at: Option<u64>,
    ) -> Symbol {
        // Get hub
        let mut hub = Self::get_hub(env.clone(), hub_id.clone()).unwrap();
        
        // Check authorization (owner only)
        let caller = env.current_contract_address();
        require_auth!(&caller);

        // Verify data entry exists
        if !hub.data_entries.iter().any(|entry| entry.id == data_entry_id) {
            panic!("Data entry not found");
        }

        // Generate permission ID
        let permission_id = symbol_short!(&format!("perm-{}", hub.permissions.len() + 1));

        let permission = Permission {
            id: permission_id.clone(),
            granter_did: hub.owner_did,
            grantee_did,
            data_entry_id,
            permission_type,
            conditions,
            created_at: env.ledger().timestamp(),
            expires_at,
            active: true,
        };

        hub.permissions.push_back(permission);
        hub.updated_at = env.ledger().timestamp();

        // Store updated hub
        let hubs_key = symbol_short!("hubs");
        let mut hubs: Map<Symbol, IdentityHub> = env
            .storage()
            .persistent()
            .get(&hubs_key)
            .unwrap_or_else(|| Map::new(&env));

        hubs.set(hub_id.clone(), hub);
        env.storage().persistent().set(&hubs_key, &hubs);

        env.events().publish(
            (extended_topics::PERM_GRANTED,),
            PermissionGrantedEvent {
                hub_id,
                permission_id: permission_id.clone(),
                grantee: env.current_contract_address(),
                grantor: env.current_contract_address(),
                timestamp: env.ledger().timestamp(),
            },
        );

        permission_id
    }

    // Revoke permission
    pub fn revoke_permission(env: Env, hub_id: Symbol, permission_id: Symbol) {
        // Get hub
        let mut hub = Self::get_hub(env.clone(), hub_id.clone()).unwrap();
        
        // Check authorization (owner only)
        let caller = env.current_contract_address();
        require_auth!(&caller);

        // Find and deactivate permission
        for mut permission in hub.permissions.iter() {
            if permission.id == permission_id {
                permission.active = false;
                break;
            }
        }

        hub.updated_at = env.ledger().timestamp();

        // Store updated hub
        let hubs_key = symbol_short!("hubs");
        let mut hubs: Map<Symbol, IdentityHub> = env
            .storage()
            .persistent()
            .get(&hubs_key)
            .unwrap_or_else(|| Map::new(&env));

        hubs.set(hub_id.clone(), hub);
        env.storage().persistent().set(&hubs_key, &hubs);

        env.events().publish(
            (extended_topics::PERM_REVOKED,),
            PermissionRevokedEvent {
                hub_id,
                permission_id,
                revoked_by: env.current_contract_address(),
                timestamp: env.ledger().timestamp(),
            },
        );
    }    // Create selective disclosure
    pub fn create_selective_disclosure(
        env: Env,
        presenter_did: Symbol,
        verifier_did: Symbol,
        data_entry_id: Symbol,
        disclosed_fields: Vec<Symbol>,
        proof: Bytes,
        nonce: Bytes,
        expires_at: u64,
    ) -> Symbol {
        // Get presenter's hub
        let hub = Self::get_hub_by_owner(env.clone(), presenter_did.clone()).unwrap();
        
        // Check authorization (presenter must own the data)
        let caller = env.current_contract_address();
        require_auth!(&caller);

        // Verify data entry exists in hub
        if !hub.data_entries.iter().any(|entry| entry.id == data_entry_id) {
            panic!("Data entry not found in presenter's hub");
        }

        // Generate disclosure ID
        let counter_key = symbol_short!("disc_cnt");
        let count: u64 = env.storage().persistent().get(&counter_key).unwrap_or(0);
        let disclosure_id = symbol_short!(&format!("disclosure-{}", count + 1));

        let disclosure = SelectiveDisclosure {
            id: disclosure_id.clone(),
            presenter_did,
            verifier_did,
            data_entry_id,
            disclosed_fields,
            proof,
            nonce,
            created_at: env.ledger().timestamp(),
            expires_at,
        };

        // Store disclosure
        let disclosures_key = symbol_short!("disclosrs");
        let mut disclosures: Map<Symbol, SelectiveDisclosure> = env
            .storage()
            .persistent()
            .get(&disclosures_key)
            .unwrap_or_else(|| Map::new(&env));

        disclosures.set(disclosure_id.clone(), disclosure);
        env.storage().persistent().set(&disclosures_key, &disclosures);
        env.storage().persistent().set(&counter_key, &(count + 1));

        env.events().publish(
            (extended_topics::DISCLOSURE_CREATED,),
            SelectiveDisclosureCreatedEvent {
                disclosure_id: disclosure_id.clone(),
                hub_id: hub.id,
                requester: env.current_contract_address(),
                timestamp: env.ledger().timestamp(),
            },
        );

        disclosure_id
    }

    // Verify selective disclosure
    pub fn verify_selective_disclosure(env: Env, disclosure_id: Symbol) -> bool {
        // Get disclosure
        let disclosure = match Self::get_disclosure(env.clone(), disclosure_id.clone()) {
            Ok(disc) => disc,
            Err(_) => return false,
        };

        // Check expiration
        if env.ledger().timestamp() > disclosure.expires_at {
            return false;
        }

        // Verify proof (simplified - in production, implement proper ZKP verification)
        if disclosure.proof.is_empty() {
            return false;
        }

        // Verify nonce uniqueness (prevent replay attacks)
        // This would require checking against a used nonce registry

        true
    }

    // Get data entry with permission check
    pub fn get_data_entry(
        env: Env,
        hub_id: Symbol,
        data_entry_id: Symbol,
        requester_did: Symbol,
    ) -> DataEntry {
        // Get hub
        let hub = Self::get_hub(env.clone(), hub_id.clone()).unwrap();

        // Find data entry
        let data_entry = hub.data_entries
            .iter()
            .find(|entry| entry.id == data_entry_id)
            .ok_or(IdentityHubError::DataEntryNotFound)
            .unwrap();

        // Check expiration
        if let Some(expiration) = data_entry.expires_at {
            if env.ledger().timestamp() > expiration {
                panic!("Data entry has expired");
            }
        }

        // Check permissions
        if hub.owner_did != requester_did {
            let has_permission = hub.permissions.iter().any(|perm| {
                perm.active &&
                perm.grantee_did == requester_did &&
                perm.data_entry_id == data_entry_id &&
                matches!(perm.permission_type, PermissionType::Read) &&
                Self::check_conditions(env.clone(), &perm.conditions)
            });

            if !has_permission {
                panic!("Permission denied");
            }
        }

        data_entry
    }

    // Get hub by ID
    pub fn get_hub_details(env: Env, hub_id: Symbol) -> IdentityHub {
        Self::get_hub(env, hub_id).unwrap()
    }

    // Get hub by owner DID
    pub fn get_hub_by_owner(env: Env, owner_did: Symbol) -> Symbol {
        let owner_map_key = symbol_short!("own_hub");
        let owner_map: Map<Symbol, Symbol> = env
            .storage()
            .persistent()
            .get(&owner_map_key)
            .ok_or(IdentityHubError::HubNotFound)?;

        owner_map.get(owner_did).ok_or(IdentityHubError::HubNotFound)
    }

    // Get hub count
    pub fn get_hub_count(env: Env) -> u64 {
        let counter_key = symbol_short!("hub_cnt");
        env.storage().persistent().get(&counter_key).unwrap_or(0)
    }

    // Internal helper methods
    fn get_hub(env: Env, hub_id: Symbol) -> Result<IdentityHub, IdentityHubError> {
        let hubs_key = symbol_short!("hubs");
        let hubs: Map<Symbol, IdentityHub> = env
            .storage()
            .persistent()
            .get(&hubs_key)
            .ok_or(IdentityHubError::HubNotFound)?;

        hubs.get(hub_id).ok_or(IdentityHubError::HubNotFound)
    }

    fn get_disclosure(env: Env, disclosure_id: Symbol) -> Result<SelectiveDisclosure, IdentityHubError> {
        let disclosures_key = symbol_short!("disclosrs");
        let disclosures: Map<Symbol, SelectiveDisclosure> = env
            .storage()
            .persistent()
            .get(&disclosures_key)
            .ok_or(IdentityHubError::DataEntryNotFound)?;

        disclosures.get(disclosure_id).ok_or(IdentityHubError::DataEntryNotFound)
    }

    fn check_conditions(env: Env, conditions: &Vec<Condition>) -> bool {
        for condition in conditions.iter() {
            // Simplified condition checking
            // In production, implement proper condition evaluation
            if condition.type_ == symbol_short!("time_lmt") {
                let current_time = env.ledger().timestamp();
                let limit = condition.value.to_u64().unwrap();
                if current_time > limit {
                    return false;
                }
            }
        }
        true
    }
}