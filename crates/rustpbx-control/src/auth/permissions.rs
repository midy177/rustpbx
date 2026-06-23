//! Role + permission model for the admin console.
//!
//! Three principal kinds:
//!   - `superadmin`   — platform operator (config account, `tenant_id = None`).
//!   - `tenant_admin` — a tenant's IAM admin; implicitly holds every permission
//!                      *within its own tenant* (incl. managing sub-users).
//!   - `tenant_user`  — a tenant sub-account limited to its granted permissions.
//!
//! Permissions are fine-grained `"<resource>:<action>"` strings. They only ever
//! constrain `tenant_user`; admins bypass the check (but are still confined to
//! their tenant by the scoping logic in `http_api`).

/// Session role strings (what `UserInfo.role` carries).
pub mod roles {
    pub const SUPERADMIN: &str = "superadmin";
    pub const TENANT_ADMIN: &str = "tenant_admin";
    pub const TENANT_USER: &str = "tenant_user";
}

/// DB-level role values stored on `rustpbx_tenant_users.role`.
pub mod db_role {
    pub const ADMIN: &str = "admin";
    pub const USER: &str = "user";
}

/// Map a stored tenant-user DB role to its session role.
pub fn session_role_for(db_role: &str) -> &'static str {
    match db_role {
        db_role::ADMIN => roles::TENANT_ADMIN,
        _ => roles::TENANT_USER,
    }
}

// ── Permission catalogue ──────────────────────────────────────────────────────

pub const TRUNKS_READ: &str = "trunks:read";
pub const TRUNKS_WRITE: &str = "trunks:write";
pub const ROUTING_READ: &str = "routing:read";
pub const ROUTING_WRITE: &str = "routing:write";
pub const EXTENSIONS_READ: &str = "extensions:read";
pub const EXTENSIONS_WRITE: &str = "extensions:write";
pub const CDR_READ: &str = "cdr:read";
pub const DIDS_READ: &str = "dids:read";
pub const DIDS_WRITE: &str = "dids:write";
pub const ACL_READ: &str = "acl:read";
pub const ACL_WRITE: &str = "acl:write";
pub const USERS_READ: &str = "users:read";
pub const USERS_WRITE: &str = "users:write";
pub const DOMAIN_READ: &str = "domain:read";
pub const DOMAIN_WRITE: &str = "domain:write";
/// View the audit trail (who changed what). Tenant admins see their own
/// tenant's entries; the superadmin sees all. Not granularly delegable.
pub const AUDIT_READ: &str = "audit:read";

/// Every permission a tenant admin can grant to a sub-user. Surfaced to the UI
/// so the permission editor stays in sync with the backend.
pub const ALL_PERMISSIONS: &[&str] = &[
    TRUNKS_READ,
    TRUNKS_WRITE,
    ROUTING_READ,
    ROUTING_WRITE,
    EXTENSIONS_READ,
    EXTENSIONS_WRITE,
    CDR_READ,
    DIDS_READ,
    DIDS_WRITE,
    ACL_READ,
    ACL_WRITE,
    USERS_READ,
    USERS_WRITE,
    DOMAIN_READ,
    DOMAIN_WRITE,
];

/// Whether a principal holds `perm`. Admins (super or tenant) always do;
/// tenant users must have it explicitly granted.
pub fn has_permission(role: &str, permissions: &[String], perm: &str) -> bool {
    match role {
        roles::SUPERADMIN | roles::TENANT_ADMIN => true,
        _ => permissions.iter().any(|p| p == perm),
    }
}

/// Validate a requested permission set against the catalogue, returning the
/// first unknown entry (if any).
pub fn first_unknown_permission(permissions: &[String]) -> Option<&str> {
    permissions
        .iter()
        .find(|p| !ALL_PERMISSIONS.contains(&p.as_str()))
        .map(|s| s.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admins_bypass_permission_checks() {
        assert!(has_permission(roles::SUPERADMIN, &[], TRUNKS_WRITE));
        assert!(has_permission(roles::TENANT_ADMIN, &[], USERS_WRITE));
    }

    #[test]
    fn tenant_user_limited_to_granted() {
        let granted = vec![TRUNKS_READ.to_string(), CDR_READ.to_string()];
        assert!(has_permission(roles::TENANT_USER, &granted, TRUNKS_READ));
        assert!(!has_permission(roles::TENANT_USER, &granted, TRUNKS_WRITE));
    }

    #[test]
    fn db_role_maps_to_session_role() {
        assert_eq!(session_role_for(db_role::ADMIN), roles::TENANT_ADMIN);
        assert_eq!(session_role_for(db_role::USER), roles::TENANT_USER);
        assert_eq!(session_role_for("anything-else"), roles::TENANT_USER);
    }

    #[test]
    fn unknown_permission_detected() {
        let p = vec![TRUNKS_READ.to_string(), "bogus:perm".to_string()];
        assert_eq!(first_unknown_permission(&p), Some("bogus:perm"));
        assert_eq!(first_unknown_permission(&[TRUNKS_READ.to_string()]), None);
    }
}
