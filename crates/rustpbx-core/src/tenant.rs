#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TenantId(pub i64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrunkContext {
    pub id: Option<i64>,
    pub name: String,
    pub tenant_id: Option<i64>,
    pub did_numbers: Vec<String>,
}
