// What leaves the device, and what the store can and cannot learn from it.
//
// Everything uploaded is one segment of the state journal, encrypted here before it goes and
// decrypted here when it comes back. The key is 32 bytes, derived once from a BIP39 recovery
// phrase in `phrase.rs`, sealed on disk beside the OAuth tokens, and uploaded nowhere.
//
// So what a Google Drive folder or an S3 bucket holds is this:
//
//   It can see    how many segments there are, how large each one is, when each was written, and
//                 the names, which carry a hash of the account's address, a device id and a range
//                 of sequence numbers. That is enough to say "somebody made about forty decisions
//                 on Tuesday, on one of their two machines", and it is the whole of it.
//   It cannot see a note, a rule, a pile, a rename, a snooze, an address, a subject or a thread
//                 key. Every one of those is inside a record, and a record only exists on the
//                 store as ciphertext.
//   It cannot lie about what it holds. Each segment is authenticated under its own name, so a
//                 store that moves a segment between devices, swaps two of them, replays an old
//                 one into a new name or flips a byte gets a decryption failure here rather than a
//                 wrong answer in somebody's mailbox.
//
// XChaCha20-Poly1305, one fresh 24 byte nonce per segment from the OS random source. The X is the
// reason it can be random rather than counted: 192 bits is wide enough that a collision is not a
// thing that happens, and there is no counter two devices could have agreed on without a server.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use rand::RngCore;

use crate::google::secrets;

const NONCE_LEN: usize = 24;

/// Four bytes in front of every segment, so a file that is not one of ours says so before the tag
/// does, and so a second format later can be told from this one rather than guessed at.
const MAGIC: &[u8; 4] = b"MMB1";

/// The sealed key's name in the token store. It is not an account id and cannot become one: a
/// Google `sub` is digits and an email address cannot carry a space.
const KEY_ID: &str = "margin-mail backup key";

/// The key, and a `Debug` that will not print it. Everything that carries one of these ends up in
/// an error message eventually.
#[derive(Clone, PartialEq, Eq)]
pub struct Key([u8; 32]);

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Key(not shown)")
    }
}

impl Key {
    pub fn from_bytes(bytes: [u8; 32]) -> Key {
        Key(bytes)
    }
}

/// Seals one segment. `name` is authenticated but not encrypted: it is the file name the caller is
/// about to write to, and binding it here is what stops the store rearranging the backup.
pub fn seal(key: &Key, name: &str, plain: &[u8]) -> Result<Vec<u8>, String> {
    let cipher = XChaCha20Poly1305::new((&key.0).into());
    let mut nonce = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce);
    let body = cipher
        .encrypt(
            (&nonce).into(),
            Payload {
                msg: plain,
                aad: name.as_bytes(),
            },
        )
        .map_err(|_| "could not encrypt a backup segment".to_string())?;

    let mut out = Vec::with_capacity(MAGIC.len() + NONCE_LEN + body.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&body);
    Ok(out)
}

/// The other direction, with the name the segment was actually found under. A wrong key, a wrong
/// name and a changed byte are all the same failure here, which is the point of an AEAD: there is
/// no partial answer to hand back and nothing that half decrypts.
pub fn open(key: &Key, name: &str, sealed: &[u8]) -> Result<Vec<u8>, String> {
    if sealed.len() < MAGIC.len() + NONCE_LEN || &sealed[..MAGIC.len()] != MAGIC {
        return Err(format!("{name} is not a Margin Mail backup segment"));
    }
    let (nonce, body) = sealed[MAGIC.len()..].split_at(NONCE_LEN);
    let nonce: &XNonce = nonce
        .try_into()
        .map_err(|_| format!("{name} is truncated"))?;
    XChaCha20Poly1305::new((&key.0).into())
        .decrypt(
            nonce,
            Payload {
                msg: body,
                aad: name.as_bytes(),
            },
        )
        .map_err(|_| {
            format!("{name} could not be decrypted. Either the recovery phrase is not the one this backup was made with, or the file has been changed since it was written.")
        })
}

// ---------------------------------------------------------------------------------------------
// The key at rest
// ---------------------------------------------------------------------------------------------
//
// `google::secrets` is the OAuth token store: an XChaCha20-Poly1305 blob in the app data
// directory, 0600 from creation, keyed from a per install salt mixed with a machine identifier so
// a copied home directory does not open it. The backup key is sealed by that code rather than
// beside it, through its own front door, because a second implementation of the same idea is a
// second thing to get wrong and it would have to be reviewed on five platforms too.
//
// Losing the sealed key is not losing the backup. The phrase derives it again.

pub fn stored() -> Result<Option<Key>, String> {
    let Some(held) = secrets::load(KEY_ID)? else {
        return Ok(None);
    };
    let raw = BASE64
        .decode(held)
        .map_err(|e| format!("the stored backup key is malformed: {e}"))?;
    let bytes: [u8; 32] = raw
        .try_into()
        .map_err(|_| "the stored backup key is not 32 bytes".to_string())?;
    Ok(Some(Key(bytes)))
}

pub fn remember(key: &Key) -> Result<(), String> {
    secrets::store(KEY_ID, &BASE64.encode(key.0))
}

pub fn forget() -> Result<(), String> {
    secrets::delete(KEY_ID)
}

/// Whether this install has a key at all, which is the same question as whether the phrase has
/// been shown yet.
pub fn have_key() -> bool {
    matches!(stored(), Ok(Some(_)))
}
