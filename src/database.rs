use rand::distr::{Alphanumeric,SampleString};
use sqlx::error;

pub fn create_login() -> Result<String ,error::Error> {
    let login = Alphanumeric.sample_string(&mut rand::rng(),10);
    Ok(login)
}

#[derive(Debug,sqlx::FromRow)]
pub struct User {
    id: String,
    login: String,
    password: String,
}

pub async fn create_db() {

}



















#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_login() {
        match create_login() {
            Ok(login) => println!("\n👉 Wygenerowany login: {}\n", login),
            Err(e) => panic!("Błąd: {:?}", e),
        }
    }
}