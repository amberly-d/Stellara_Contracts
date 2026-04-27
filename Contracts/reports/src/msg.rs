use serde::{Deserialize, Serialize};
use crate::state::Filter;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum ExecuteMsg {
    CreateReport {
        name: String,
        description: String,
        data_source: String,
        filters: Vec<Filter>,
        group_by: Vec<String>,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum QueryMsg {
    GetReport { id: u64 },
}
