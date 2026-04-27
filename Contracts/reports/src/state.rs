use cosmwasm_std::Addr;
use cw_storage_plus::{Item, Map};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ReportTemplate {
    pub id: u64,
    pub owner: Addr,
    pub name: String,
    pub description: String,
    pub data_source: String,
    pub filters: Vec<Filter>,
    pub group_by: Vec<String>,
    pub created_at: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Filter {
    pub field: String,
    pub operator: String, // "eq", "gt", etc.
    pub value: String,
}

pub const REPORTS: Map<u64, ReportTemplate> = Map::new("reports");
pub const REPORT_COUNT: Item<u64> = Item::new("report_count");
