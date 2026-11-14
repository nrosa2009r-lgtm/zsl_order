// SPDX-License-Identifier: MIT
//! Jedno źródło prawdy dla polityki haseł.
//!
//! Minimum długości jest używane jednocześnie przez walidację na serwerze,
//! komunikaty błędów i interfejs (pole `password_min_length` w API). Dzięki
//! temu komunikat nie może rozjechać się z rzeczywistą regułą.

/// Minimalna długość hasła dla konta ucznia.
pub const STUDENT_MIN_LENGTH: usize = 10;

/// Minimalna długość hasła dla kuchni i administratora.
pub const STAFF_MIN_LENGTH: usize = 12;

/// Górny limit długości hasła (ochrona przed nadmiernie długim wejściem).
pub const MAX_LENGTH: usize = 128;

/// Minimalna długość hasła dla roli. Nieznana rola traktowana jest jak
/// personel — lepiej wymagać więcej niż mniej.
pub fn min_length(role: &str) -> usize {
    if role == "student" {
        STUDENT_MIN_LENGTH
    } else {
        STAFF_MIN_LENGTH
    }
}

/// Komunikat o wymaganiach hasła, spójny z `min_length`.
pub fn requirements_text(role: &str) -> String {
    format!(
        "Hasło musi mieć co najmniej {} znaków. Nie może zawierać loginu ani być zbyt popularne.",
        min_length(role)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimum_depends_on_role() {
        assert_eq!(min_length("student"), 10);
        assert_eq!(min_length("kitchen"), 12);
        assert_eq!(min_length("admin"), 12);
        // Nieznana rola nie może dostać słabszego progu niż personel.
        assert_eq!(min_length(""), 12);
    }

    #[test]
    fn requirements_text_uses_minimum_from_the_same_constant() {
        for role in ["student", "kitchen", "admin"] {
            let min = min_length(role);
            let text = requirements_text(role);
            assert!(
                text.contains(&min.to_string()),
                "komunikat dla roli {role} nie zawiera {min}: {text}"
            );
        }
    }

    #[test]
    fn frontend_takes_the_minimum_from_the_api() {
        let js = include_str!("../frontend/common.js");
        assert!(
            js.contains("data.password_min_length"),
            "panel musi czytać minimum z /api/auth/me"
        );
        assert!(
            js.contains("data.password_requirements"),
            "panel musi używać komunikatu z serwera"
        );
        assert!(
            !js.contains("co najmniej 10 znaków"),
            "panel nie może zaszywać minimalnej długości"
        );
        assert!(
            !js.contains("literę i cyfrę"),
            "komunikat nie może obiecywać reguły, której walidacja nie wymaga"
        );
    }
}
