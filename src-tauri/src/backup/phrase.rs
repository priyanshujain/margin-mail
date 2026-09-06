// The twenty-four words, and the only way back into a backup.
//
// The phrase is generated once, on the device, when backup is first turned on. It is shown once
// and then it is gone: what stays behind is the key it derives, sealed by `crypto`, and Argon2 is
// one way, so this app cannot show the phrase again even if a screen asked it to. That is the
// property being bought. A phrase the app can reprint is a phrase the app is storing, and a phrase
// the app is storing is one more file that has to be as well defended as the backup itself.
//
// BIP39 rather than a random string because the words are transcribable. This gets written on
// paper, read out over a phone call and typed on a machine that is not the one it came from, and
// the wordlist carries a checksum that catches a wrong word at the point of typing rather than as
// a decryption failure ten seconds later.

use argon2::{Algorithm, Argon2, Params, Version};
use bip39::Mnemonic;

use super::crypto::Key;

/// Twenty-four words, so 256 bits of entropy. Twelve would be past anything anyone can brute force
/// too, and the extra dozen words cost one more line on the piece of paper.
pub const WORDS: usize = 24;

/// Argon2id, 64 MiB, three passes, one lane, 32 bytes out.
///
/// Argon2id because it is the hybrid: 2i alone gives up GPU resistance and 2d alone is open to a
/// side channel, and RFC 9106 tells anyone without a specific reason to differ to use the hybrid.
///
/// The cost is RFC 9106's second recommended configuration, the one written for a machine that
/// cannot spare a gigabyte, which is the right shape for something that has to run on a phone. It
/// is a fraction of a second on a laptop and under a second on a handset, and it runs exactly
/// twice in the life of an installation: once at setup and once at a restore. There is no
/// interactive cost to trade it against, so there was no reason to go lower.
///
/// One lane rather than the four the RFC pairs with 64 MiB because this implementation walks the
/// lanes in sequence rather than in threads. Four lanes would quarter the memory each one touches
/// and shorten nothing, and the ratio between what this costs us and what it costs an attacker is
/// the same either way.
const M_COST_KIB: u32 = 64 * 1024;
const T_COST: u32 = 3;
const LANES: u32 = 1;

/// A constant, which for a password would be a bug and here is forced. A second device has the
/// phrase and nothing else, so every input to the derivation has to be reachable from the phrase
/// alone; a random salt would have to be stored somewhere, and the only place to store it is the
/// backup, which cannot be read until the key exists. What a per user salt buys is that one
/// precomputed table cannot answer for many users, and there is nothing to precompute against 256
/// bits from the wordlist. The entropy is doing the work here and Argon2 is the belt to it.
const SALT: &[u8] = b"margin-mail backup phrase v1";

/// A fresh phrase. Never returned twice: the caller shows it, seals the key it derives, and this
/// module keeps nothing.
pub fn generate() -> Result<String, String> {
    Mnemonic::generate(WORDS)
        .map(|mnemonic| mnemonic.to_string())
        .map_err(|e| format!("could not make a recovery phrase: {e}"))
}

/// The phrase as the derivation sees it: the wordlist's own spelling, single spaced. Typing it in
/// a different case, with an extra space or with a line break in the middle is the same phrase, and
/// a mistyped word is refused here by name rather than becoming a wrong key and an unreadable
/// backup twenty seconds later.
///
/// The case folding is not politeness. This gets typed on the phone that the backup is being
/// restored onto, every soft keyboard capitalises the first word of what looks like a sentence, and
/// the wordlist is lower case, so without this the commonest way of entering a phrase correctly
/// fails as if the phrase were wrong.
pub fn normalise(phrase: &str) -> Result<String, String> {
    let tidied = phrase
        .split_whitespace()
        .map(|word| word.to_lowercase())
        .collect::<Vec<String>>()
        .join(" ");
    Mnemonic::parse(&tidied)
        .map(|mnemonic| mnemonic.to_string())
        .map_err(|e| format!("that is not a recovery phrase: {e}"))
}

pub fn derive(phrase: &str) -> Result<Key, String> {
    let normalised = normalise(phrase)?;
    let params = Params::new(M_COST_KIB, T_COST, LANES, Some(32))
        .map_err(|e| format!("the key derivation is misconfigured: {e}"))?;
    let mut out = [0u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(normalised.as_bytes(), SALT, &mut out)
        .map_err(|e| format!("could not derive the backup key: {e}"))?;
    Ok(Key::from_bytes(out))
}
