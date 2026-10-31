use serde_json::Value;
use std::{fs, io,};

pub fn write_config_parm(
    path: &str,
    parm: &str,
    value: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut json_value: Value = match fs::read_to_string(path) {
        Ok(data) => serde_json::from_str(&data)?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => Value::Object(Default::default()),
        Err(e) => return Err(e.into()),
    };

    if let Some(obj) = json_value.as_object_mut() {
        obj.insert(parm.to_string(), Value::from(value));
    }

    fs::write(path, serde_json::to_string_pretty(&json_value)?)?;
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