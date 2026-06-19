use soroban_sdk::{Address, Env, Map, Symbol, Vec, symbol_short};

// Standard Roles
pub const ROLE_ADMIN: Symbol = symbol_short!("admin");
pub const ROLE_APPROVER: Symbol = symbol_short!("approver");
pub const ROLE_EXECUTOR: Symbol = symbol_short!("executor");
pub const ROLE_USER: Symbol = symbol_short!("user");

// Standard Permissions
pub const PERMISSION_SET_RATE: Symbol = symbol_short!("set_rate");
pub const PERMISSION_PREMIUM: Symbol = symbol_short!("premium");
pub const PERMISSION_PAUSE: Symbol = symbol_short!("pause");
pub const PERMISSION_UNPAUSE: Symbol = symbol_short!("unpause");
pub const PERMISSION_MGR_ACL: Symbol = symbol_short!("mgr_acl");
pub const PERMISSION_NEW_POOL: Symbol = symbol_short!("new_pool");
pub const PERMISSION_PROPOSE: Symbol = symbol_short!("propose");
pub const PERMISSION_APPROVE: Symbol = symbol_short!("approve");
pub const PERMISSION_EXECUTE: Symbol = symbol_short!("execute");

pub struct ACL;

impl ACL {
    fn user_roles_key(env: &Env) -> Symbol {
        Symbol::new(env, "u_roles")
    }

    fn role_perms_key(env: &Env) -> Symbol {
        Symbol::new(env, "r_perms")
    }

    fn role_parent_key(env: &Env) -> Symbol {
        Symbol::new(env, "r_parent")
    }

    fn roles_exist_key(env: &Env) -> Symbol {
        Symbol::new(env, "roles_ex")
    }

    pub fn create_role(env: &Env, role: &Symbol) {
        let key = Self::roles_exist_key(env);
        let mut roles: Map<Symbol, bool> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or(Map::new(env));

        roles.set(role.clone(), true);
        env.storage().persistent().set(&key, &roles);
    }

    pub fn role_exists(env: &Env, role: &Symbol) -> bool {
        let key = Self::roles_exist_key(env);
        let roles: Map<Symbol, bool> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or(Map::new(env));

        roles.get(role.clone()).unwrap_or(false)
    }

    pub fn assign_role(env: &Env, user: &Address, role: &Symbol) {
        if !Self::role_exists(env, role) {
            Self::create_role(env, role);
        }

        let key = Self::user_roles_key(env);
        let mut roles: Map<Address, Vec<Symbol>> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or(Map::new(env));

        let mut user_roles = roles.get(user.clone()).unwrap_or(Vec::new(env));

        if !user_roles.iter().any(|r| r == role.clone()) {
            user_roles.push_back(role.clone());
        }

        roles.set(user.clone(), user_roles);
        env.storage().persistent().set(&key, &roles);
    }

    pub fn assign_permission(env: &Env, role: &Symbol, permission: &Symbol) {
        if !Self::role_exists(env, role) {
            Self::create_role(env, role);
        }

        let key = Self::role_perms_key(env);
        let mut perms: Map<Symbol, Vec<Symbol>> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or(Map::new(env));

        let mut role_perms = perms.get(role.clone()).unwrap_or(Vec::new(env));

        if !role_perms.iter().any(|p| p == permission.clone()) {
            role_perms.push_back(permission.clone());
        }

        perms.set(role.clone(), role_perms);
        env.storage().persistent().set(&key, &perms);
    }

    pub fn assign_permissions_batch(env: &Env, role: &Symbol, permissions: &Vec<Symbol>) {
        if !Self::role_exists(env, role) {
            Self::create_role(env, role);
        }

        for permission in permissions.iter() {
            Self::assign_permission(env, role, &permission);
        }
    }

    pub fn set_parent_role(env: &Env, child: &Symbol, parent: &Symbol) {
        if child == parent {
            panic!("INVALID_ROLE_HIERARCHY");
        }

        if !Self::role_exists(env, child) {
            Self::create_role(env, child);
        }

        if !Self::role_exists(env, parent) {
            Self::create_role(env, parent);
        }

        let key = Self::role_parent_key(env);
        let mut parents: Map<Symbol, Symbol> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or(Map::new(env));

        parents.set(child.clone(), parent.clone());
        env.storage().persistent().set(&key, &parents);
    }

    pub fn get_parent_role(env: &Env, role: &Symbol) -> Option<Symbol> {
        let parents: Map<Symbol, Symbol> = env
            .storage()
            .persistent()
            .get(&Self::role_parent_key(env))
            .unwrap_or(Map::new(env));

        parents.get(role.clone())
    }

    pub fn has_permission(env: &Env, user: &Address, permission: &Symbol) -> bool {
        let roles: Map<Address, Vec<Symbol>> = env
            .storage()
            .persistent()
            .get(&Self::user_roles_key(env))
            .unwrap_or(Map::new(env));

        let perms: Map<Symbol, Vec<Symbol>> = env
            .storage()
            .persistent()
            .get(&Self::role_perms_key(env))
            .unwrap_or(Map::new(env));

        let parents: Map<Symbol, Symbol> = env
            .storage()
            .persistent()
            .get(&Self::role_parent_key(env))
            .unwrap_or(Map::new(env));

        let user_roles = roles.get(user.clone()).unwrap_or(Vec::new(env));

        for role in user_roles.iter() {
            if Self::check_role(env, &perms, &parents, role.clone(), permission, 0) {
                return true;
            }
        }

        false
    }

    fn check_role(
        env: &Env,
        perms: &Map<Symbol, Vec<Symbol>>,
        parents: &Map<Symbol, Symbol>,
        role: Symbol,
        permission: &Symbol,
        depth: u32,
    ) -> bool {
        if depth > 10 {
            panic!("ROLE_HIERARCHY_TOO_DEEP");
        }

        let role_perms = perms.get(role.clone()).unwrap_or(Vec::new(env));

        for p in role_perms.iter() {
            if p == permission.clone() {
                return true;
            }
        }

        if let Some(parent) = parents.get(role) {
            return Self::check_role(env, perms, parents, parent, permission, depth + 1);
        }

        false
    }

    pub fn require_permission(env: &Env, user: &Address, permission: &Symbol) {
        if !Self::has_permission(env, user, permission) {
            panic!("UNAUTHORIZED");
        }
    }

    pub fn get_user_roles(env: &Env, user: &Address) -> Vec<Symbol> {
        let roles: Map<Address, Vec<Symbol>> = env
            .storage()
            .persistent()
            .get(&Self::user_roles_key(env))
            .unwrap_or(Map::new(env));

        roles.get(user.clone()).unwrap_or(Vec::new(env))
    }

    pub fn get_role_permissions(env: &Env, role: &Symbol) -> Vec<Symbol> {
        let perms: Map<Symbol, Vec<Symbol>> = env
            .storage()
            .persistent()
            .get(&Self::role_perms_key(env))
            .unwrap_or(Map::new(env));

        perms.get(role.clone()).unwrap_or(Vec::new(env))
    }
}
