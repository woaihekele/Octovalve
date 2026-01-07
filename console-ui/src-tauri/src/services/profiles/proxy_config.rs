use std::fs;
use std::path::Path;

use toml_edit::{value, DocumentMut, Item, Table};

pub(crate) fn sync_proxy_config_runtime_ports(
    path: &Path,
    listen_port: u16,
    control_port: u16,
) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }

    let original = fs::read_to_string(path).map_err(|err| err.to_string())?;
    let mut doc = original
        .parse::<DocumentMut>()
        .map_err(|err| err.to_string())?;

    ensure_defaults_table(&mut doc)?;
    let defaults = doc["defaults"]
        .as_table_mut()
        .ok_or_else(|| "proxy config defaults must be a table".to_string())?;

    defaults["remote_addr"] = value(format!("127.0.0.1:{listen_port}"));
    defaults["control_remote_addr"] = value(format!("127.0.0.1:{control_port}"));

    if let Some(targets) = doc["targets"].as_array_of_tables_mut() {
        for target in targets.iter_mut() {
            if target.contains_key("remote_addr") {
                target["remote_addr"] = value(format!("127.0.0.1:{listen_port}"));
            }
            if target.contains_key("control_remote_addr") {
                target["control_remote_addr"] = value(format!("127.0.0.1:{control_port}"));
            }
        }
    }

    let updated = doc.to_string();
    if updated != original {
        fs::write(path, updated).map_err(|err| err.to_string())?;
    }
    Ok(())
}

fn ensure_defaults_table(doc: &mut DocumentMut) -> Result<(), String> {
    if doc.get("defaults").is_none() {
        doc["defaults"] = Item::Table(Table::new());
    }
    if !doc["defaults"].is_table() {
        return Err("proxy config defaults must be a table".to_string());
    }
    Ok(())
}
