//! A shared world's invite: one line of text that holds where the world's bucket is, the key
//! that opens it, and which world in it. Whoever has the line is a member of the world, so it
//! is sent to friends only; it is not the short-lived code that joins a host.
use crate::{
    Error, Result,
    codec::{base64, hex, random, sha256, unbase64},
    s3::Bucket,
};
use serde::{Deserialize, Serialize};

/// What every invite starts with: the format and its version.
pub const PREFIX: &str = "HSW1.";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldInvite {
    pub bucket: Bucket,
    /// The world's id: 16 hex digits, the name of its folder in the bucket.
    pub world: String,
    /// The world's name as its players see it.
    pub name: String,
}

fn valid_id(id: &str) -> bool {
    id.len() == 16 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

impl WorldInvite {
    /// A new world's id.
    pub fn new_id() -> Result<String> {
        Ok(hex(&random::<8>()?))
    }

    /// The world's folder in the bucket: `w/<id>/`.
    pub fn prefix(&self) -> String {
        format!("w/{}/", self.world)
    }

    fn check(&self) -> Result<()> {
        if !valid_id(&self.world) {
            return Err(Error::Format("the invite names no world".into()));
        }
        let name = self.name.trim();
        if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
            return Err(Error::Format(
                "the invite's world has no usable name".into(),
            ));
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<String> {
        self.check()?;
        let mut data = serde_json::to_vec(self)?;
        let sum = sha256(&data);
        data.extend_from_slice(&sum[..4]);
        Ok(format!("{PREFIX}{}", base64(&data)))
    }

    /// Reads an invite as pasted: spaces and line breaks a chat app added are ignored, and a
    /// line that was cut or mistyped is caught by its checksum.
    pub fn decode(text: &str) -> Result<Self> {
        let text: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        let body = text
            .strip_prefix(PREFIX)
            .ok_or_else(|| Error::Format("this is not a shared world invite".into()))?;
        let cut = || Error::Format("the invite is incomplete: copy the whole line".into());
        let data = unbase64(body).map_err(|_| cut())?;
        let Some((json, sum)) = data.split_last_chunk::<4>() else {
            return Err(cut());
        };
        if sha256(json)[..4] != sum[..] {
            return Err(cut());
        }
        let invite: Self = serde_json::from_slice(json)?;
        invite.check()?;
        Ok(invite)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invite() -> WorldInvite {
        WorldInvite {
            bucket: Bucket {
                endpoint: "https://0123456789abcdef0123456789abcdef.r2.cloudflarestorage.com"
                    .into(),
                bucket: "our-worlds".into(),
                region: "auto".into(),
                access_key: "0123456789abcdef0123456789abcdef".into(),
                secret_key: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                    .into(),
            },
            world: WorldInvite::new_id().unwrap(),
            name: "The Island".into(),
        }
    }

    #[test]
    fn an_invite_reads_back_however_a_chat_wrapped_it() {
        let invite = invite();
        let text = invite.encode().unwrap();
        assert!(text.starts_with(PREFIX));
        assert!(text.len() < 600, "{} characters", text.len());
        assert_eq!(WorldInvite::decode(&text).unwrap(), invite);
        let wrapped: String = text
            .as_bytes()
            .chunks(40)
            .map(|line| format!("  {}\r\n", std::str::from_utf8(line).unwrap()))
            .collect();
        assert_eq!(WorldInvite::decode(&wrapped).unwrap(), invite);
        assert_eq!(invite.prefix(), format!("w/{}/", invite.world));
    }

    #[test]
    fn a_cut_or_changed_invite_is_refused() {
        let text = invite().encode().unwrap();
        assert!(WorldInvite::decode(&text[..text.len() - 3]).is_err());
        assert!(WorldInvite::decode(&text[PREFIX.len()..]).is_err());
        assert!(WorldInvite::decode("").is_err());
        assert!(WorldInvite::decode(PREFIX).is_err());
        // One character changed in the middle.
        let mut changed = text.clone().into_bytes();
        let middle = changed.len() / 2;
        changed[middle] = if changed[middle] == b'A' { b'B' } else { b'A' };
        assert!(WorldInvite::decode(std::str::from_utf8(&changed).unwrap()).is_err());
    }

    #[test]
    fn an_invite_without_a_world_or_a_name_is_not_made() {
        let mut bad = invite();
        bad.world = "../lease".into();
        assert!(bad.encode().is_err());
        let mut bad = invite();
        bad.name = "  ".into();
        assert!(bad.encode().is_err());
        let ids: std::collections::HashSet<String> =
            (0..50).map(|_| WorldInvite::new_id().unwrap()).collect();
        assert_eq!(ids.len(), 50);
        assert!(ids.iter().all(|id| valid_id(id)));
    }
}
