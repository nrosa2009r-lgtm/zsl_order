use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use rand::Rng;
use rand_core::OsRng;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::Path;

/// Plik z sekretem HMAC używanym do podpisywania tokenów JWT.
pub const JWT_SECRET_FILE: &str = "jwt.key";

/// Alfabet bez znaków mylonych (0/O, 1/l/I).
pub const SAFE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
const DIGITS: &[u8] = b"23456789";
const UPPER: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ";

/// Hashowanie hasła Argon2id z losową solą.
pub fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
    let salt = SaltString::generate(&mut OsRng);
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)?
        .to_string())
}

/// Weryfikacja hasła. Argon2 porównuje skróty w czasie stałym.
pub fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .map(|parsed| {
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok()
        })
        .unwrap_or(false)
}

/// Kryptograficznie bezpieczny ciąg znaków z bezpiecznego alfabetu.
pub fn random_string(len: usize) -> String {
    let mut rng = rand::rng();
    (0..len)
        .map(|_| {
            let idx = rng.random_range(0..SAFE_ALPHABET.len());
            SAFE_ALPHABET[idx] as char
        })
        .collect()
}

/// Hasło z gwarancją liter i cyfr, losowo przetasowane.
pub fn random_password(len: usize) -> String {
    let len = len.max(8);
    let mut chars: Vec<char> = random_string(len).chars().collect();
    let mut rng = rand::rng();
    chars[0] = UPPER[rng.random_range(0..UPPER.len())] as char;
    chars[1] = DIGITS[rng.random_range(0..DIGITS.len())] as char;

    // Fisher-Yates, aby pozycje znaków nie były przewidywalne.
    for i in (1..chars.len()).rev() {
        let j = rng.random_range(0..=i);
        chars.swap(i, j);
    }
    chars.into_iter().collect()
}

/// Login w formacie `s-abcdefgh12` itp.
pub fn generate_login(prefix: &str) -> String {
    format!("{}-{}", prefix, random_string(10).to_lowercase())
}

/// Krótki, publiczny identyfikator (bez danych osobowych).
pub fn generate_display_code(prefix: &str) -> String {
    format!("{}-{}", prefix, random_string(5).to_uppercase())
}

/// Kod odbioru zamówienia, np. `K7M2QP`.
pub fn generate_pickup_code(len: usize) -> String {
    let mut rng = rand::rng();
    (0..len)
        .map(|_| {
            let idx = rng.random_range(0..UPPER.len());
            UPPER[idx] as char
        })
        .collect()
}

/// Losowe bajty w postaci hex (używane do tokenów sesji i QR).
pub fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::rng().fill(buf.as_mut_slice());
    hex::encode(buf)
}

/// Sesje przechowujemy jako SHA-256 tokenu — wyciek bazy nie ujawnia
/// aktywnych ciasteczek.
pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

/// Wczytuje sekret JWT z pliku albo tworzy go bezpiecznie przy pierwszym
/// uruchomieniu. Plik ma uprawnienia `0600` i nie jest wersjonowany.
///
/// Sekret ma postać co najmniej 64 znaków hex (32 bajty entropii).
/// Usunięcie pliku unieważnia wszystkie wydane tokeny JWT.
pub fn load_or_create_jwt_secret(path: &Path) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    match fs::read_to_string(path) {
        Ok(text) => {
            // Plik mógł zostać utworzony lub skopiowany z szerszymi
            // uprawnieniami — zawsze wymuszamy 0600.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
            }

            let secret = text.trim();
            if secret.len() < 64 || !secret.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(format!(
                    "Plik {} nie zawiera poprawnego sekretu JWT (wymagane min. 64 znaki hex).",
                    path.display()
                )
                .into());
            }
            Ok(secret.as_bytes().to_vec())
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            let secret = random_hex(48);

            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }

            match options.open(path) {
                Ok(mut file) => {
                    file.write_all(secret.as_bytes())?;
                    file.write_all(b"\n")?;
                    file.sync_all()?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
                    }
                    Ok(secret.into_bytes())
                }
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                    // Inny proces utworzył plik w międzyczasie — wczytaj go.
                    let text = fs::read_to_string(path)?;
                    let secret = text.trim();
                    if secret.len() < 64 || !secret.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                        return Err(format!(
                            "Plik {} nie zawiera poprawnego sekretu JWT.",
                            path.display()
                        )
                        .into());
                    }
                    Ok(secret.as_bytes().to_vec())
                }
                Err(error) => Err(error.into()),
            }
        }
        Err(error) => Err(error.into()),
    }
}

/// Porównanie w czasie stałym dla tokenów CSRF.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_hash_roundtrip() {
        let hash = hash_password("PrzykladoweHaslo123").unwrap();
        assert!(verify_password("PrzykladoweHaslo123", &hash));
        assert!(!verify_password("ZleHaslo123", &hash));
    }

    #[test]
    fn generated_password_has_letter_and_digit() {
        for _ in 0..50 {
            let pass = random_password(14);
            assert_eq!(pass.chars().count(), 14);
            assert!(pass.chars().any(|c| c.is_ascii_alphabetic()));
            assert!(pass.chars().any(|c| c.is_ascii_digit()));
        }
    }

    #[test]
    fn tokens_are_random_and_hashable() {
        let a = random_hex(32);
        let b = random_hex(32);
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        assert_eq!(hash_token(&a), hash_token(&a));
        assert_ne!(hash_token(&a), hash_token(&b));
    }

    #[test]
    fn constant_time_compare() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }

    #[test]
    fn jwt_secret_is_created_and_reused() {
        let path = std::env::temp_dir().join(format!("zsl-order-jwt-test-{}.key", random_hex(8)));
        let first = load_or_create_jwt_secret(&path).expect("tworzenie sekretu");
        let second = load_or_create_jwt_secret(&path).expect("odczyt sekretu");
        assert_eq!(first, second);
        assert!(first.len() >= 64);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
            let third = load_or_create_jwt_secret(&path).expect("odczyt sekretu 0644");
            assert_eq!(first, third);
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(
                mode, 0o600,
                "istniejący plik musi zostać utwardzony do 0600"
            );
        }

        let _ = fs::remove_file(path);
    }
}
