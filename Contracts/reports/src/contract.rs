use cosmwasm_std::{DepsMut, Env, Event, MessageInfo, Response};
use crate::state::{REPORTS, REPORT_COUNT, ReportTemplate};
use crate::msg::ExecuteMsg;

pub fn execute_create_report(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    name: String,
    description: String,
    data_source: String,
    filters: Vec<crate::state::Filter>,
    group_by: Vec<String>,
) -> Result<Response, ContractError> {
    let mut count = REPORT_COUNT.load(deps.storage).unwrap_or(0);
    count += 1;

    let report = ReportTemplate {
        id: count,
        owner: info.sender.clone(),
        name: name.clone(),
        description,
        data_source: data_source.clone(),
        filters,
        group_by,
        created_at: env.block.time.seconds(),
    };

    REPORTS.save(deps.storage, count, &report)?;
    REPORT_COUNT.save(deps.storage, &count)?;

    Ok(Response::new()
        .add_attribute("action", "create_report")
        .add_attribute("report_id", count.to_string())
        .add_attribute("owner", info.sender.to_string())
        .add_event(
            Event::new("report_created")
                .add_attribute("report_id", count.to_string())
                .add_attribute("owner", info.sender.to_string())
                .add_attribute("data_source", data_source),
        ))
}