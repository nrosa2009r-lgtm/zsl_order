use serde_json::Value;
use std::error::Error;
use std::{fs, io};

use serde_json::json;
use std::fs::File;
use std::io::Write;

use crate::CONF_PATH;
pub fn create_config_file() -> Result<(), Box<dyn Error>> {
    let config_parms = json!({
    "ip": "192.168.0.124",
    "port": 3000,
    "status": "working",
    "max_connections": "120",
    "db_url": "baza.db"
      });

    let steralised_data = serde_json::to_string_pretty(&config_parms)?;

    let mut cof_file = File::create(CONF_PATH)?;

    cof_file.write_all(steralised_data.as_bytes())?;

    Ok(())
}

pub fn get_config_data(path: &str, key: &str) -> Result<String, io::Error> {
    let file = fs::File::open(path)?;
    let v: Value =
        serde_json::from_reader(file).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    match v.get(key) {
        Some(value) => Ok(value
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| value.to_string())),
        None => Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Klucz '{key}' nie został znaleziony w pliku konfiguracyjnym"),
        )),
    }
}
