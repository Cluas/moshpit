//! What the server asks at sign-in that only the person can answer
//! (keyboard-interactive, RFC 4256): a one-time code, a second factor, a
//! password asked again. The saved password answers only a prompt that
//! reads as one ([`reads_as_password`]); every other prompt goes to the
//! person with the server's own words, through [`SignInQuestions`].

use crate::host_key::BoxFuture;

/// One prompt of a keyboard-interactive round, as the server words it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignInQuestion {
    /// The round's name and instructions (often empty).
    pub name: String,
    pub instructions: String,
    /// The prompt itself: "Verification code: ".
    pub prompt: String,
    /// Whether the answer is shown as typed (false: a secret).
    pub echo: bool,
}

/// The person, answering a [`SignInQuestion`]: `None` when they decline.
pub trait SignInQuestions: Send + Sync {
    fn ask<'a>(
        &'a self,
        host: &'a str,
        question: &'a SignInQuestion,
    ) -> BoxFuture<'a, Option<String>>;
}

/// The words a prompt for the account's password uses, lowercased: the
/// English ones, then what Linux-PAM's own translations and the BSDs' say
/// ("Password:" in de, fr, es, it, pt, nl, pl, sv, da, nb, fi, cs, tr, ru,
/// uk, zh_CN, zh_TW, ja, ko). Matched anywhere in the prompt, so "user's
/// password:" and "Passwort für me:" count. A one-time code, a token, a
/// PIN are none of these: they go to the person (B563).
const PASSWORD_WORDS: &[&str] = &[
    "password",
    "passphrase",
    "passwort",
    "kennwort",
    "mot de passe",
    "contraseña",
    "contrasena",
    "senha",
    "palavra-passe",
    "wachtwoord",
    "hasło",
    "haslo",
    "lösenord",
    "adgangskode",
    "passord",
    "salasana",
    "heslo",
    "parola",
    "şifre",
    "sifre",
    "пароль",
    "密码",
    "密碼",
    "口令",
    "パスワード",
    "비밀번호",
    "암호",
];

/// What makes a prompt that names a password ask for something else: a
/// one-time password (pam_oath's "One-time password (OATH) for `me':"),
/// a verification code, a token.
const CODE_WORDS: &[&str] = &[
    "one-time",
    "one time",
    "otp",
    "oath",
    "verification",
    "code",
    "token",
];

/// Whether `prompt` asks for the account's password: it names one
/// ([`PASSWORD_WORDS`]) and no code ([`CODE_WORDS`]).
pub fn reads_as_password(prompt: &str) -> bool {
    let prompt = prompt.to_lowercase();
    PASSWORD_WORDS.iter().any(|word| prompt.contains(word))
        && !CODE_WORDS.iter().any(|word| prompt.contains(word))
}

#[cfg(test)]
mod tests {
    use super::reads_as_password;

    #[test]
    fn password_prompts_read_as_such_and_codes_do_not() {
        for yes in [
            "Password: ",
            "me@box's password: ",
            "Passwort: ",
            "Mot de passe : ",
            "密码：",
            "パスワード:",
            "비밀번호:",
            "Пароль: ",
            "Enter passphrase for key: ",
        ] {
            assert!(reads_as_password(yes), "{yes}");
        }
        for no in [
            "Verification code: ",
            "One-time password (OATH) for `me': ",
            "Enter PIN: ",
            "Token: ",
            "Duo two-factor login: ",
        ] {
            assert!(!reads_as_password(no), "{no}");
        }
    }
}
