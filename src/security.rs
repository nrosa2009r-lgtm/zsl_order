use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
pub fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
    let salt: SaltString = SaltString::generate(&mut OsRng);

    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)?
        .to_string())
}

pub fn veryfy_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .map(|prased| {
            Argon2::default()
                .verify_password(password.as_bytes(), &prased)
                .is_ok()
        })
        .unwrap_or(false)
}
