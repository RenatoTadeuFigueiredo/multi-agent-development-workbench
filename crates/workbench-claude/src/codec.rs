use std::{collections::HashSet, fmt};

use serde::{
    Deserialize, Deserializer,
    de::{Error as _, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Number, Value};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{ClaudeError, ClaudeErrorKind, MAX_FRAME_BYTES};

pub(crate) struct FrameReader<R> {
    reader: R,
    buffered: Vec<u8>,
    searched_until: usize,
}

impl<R: AsyncRead + Unpin> FrameReader<R> {
    pub(crate) fn new(reader: R) -> Self {
        Self {
            reader,
            buffered: Vec::new(),
            searched_until: 0,
        }
    }

    pub(crate) async fn next_frame(&mut self) -> Result<Option<Value>, ClaudeError> {
        loop {
            if let Some(relative_newline) = self.buffered[self.searched_until..]
                .iter()
                .position(|byte| *byte == b'\n')
            {
                let newline = self.searched_until + relative_newline;
                if newline > MAX_FRAME_BYTES {
                    return Err(frame_too_large());
                }
                let frame = self.buffered.drain(..=newline).collect::<Vec<_>>();
                self.searched_until = 0;
                return decode_frame(&frame[..newline]).map(Some);
            }
            self.searched_until = self.buffered.len();
            if self.buffered.len() > MAX_FRAME_BYTES {
                return Err(frame_too_large());
            }
            let mut chunk = vec![0_u8; 16 * 1024];
            let read = self
                .reader
                .read(&mut chunk)
                .await
                .map_err(|_| transport_closed())?;
            if read == 0 {
                if self.buffered.is_empty() {
                    return Ok(None);
                }
                return Err(invalid_frame());
            }
            self.buffered.extend_from_slice(&chunk[..read]);
        }
    }
}

pub(crate) async fn write_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    value: &Value,
) -> Result<(), ClaudeError> {
    let encoded = encode_frame(value)?;
    writer
        .write_all(&encoded)
        .await
        .map_err(|_| transport_closed())?;
    writer
        .write_all(b"\n")
        .await
        .map_err(|_| transport_closed())?;
    writer.flush().await.map_err(|_| transport_closed())
}

pub(crate) fn encode_frame(value: &Value) -> Result<Vec<u8>, ClaudeError> {
    let encoded = serde_json::to_vec(value).map_err(|_| invalid_frame())?;
    if encoded.len() > MAX_FRAME_BYTES {
        return Err(frame_too_large());
    }
    Ok(encoded)
}

pub(crate) fn decode_frame(bytes: &[u8]) -> Result<Value, ClaudeError> {
    if bytes.is_empty() || bytes.len() > MAX_FRAME_BYTES {
        return if bytes.len() > MAX_FRAME_BYTES {
            Err(frame_too_large())
        } else {
            Err(invalid_frame())
        };
    }
    let text = std::str::from_utf8(bytes).map_err(|_| invalid_frame())?;
    if text.trim().is_empty() {
        return Err(invalid_frame());
    }
    let mut deserializer = serde_json::Deserializer::from_str(text);
    let UniqueValue(value) =
        UniqueValue::deserialize(&mut deserializer).map_err(|_| invalid_frame())?;
    deserializer.end().map_err(|_| invalid_frame())?;
    Ok(value)
}

struct UniqueValue(Value);

impl<'de> Deserialize<'de> for UniqueValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(UniqueValueVisitor)
    }
}

struct UniqueValueVisitor;

impl<'de> Visitor<'de> for UniqueValueVisitor {
    type Value = UniqueValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(UniqueValue(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(UniqueValue(Value::Number(Number::from(value))))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(UniqueValue(Value::Number(Number::from(value))))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Number::from_f64(value)
            .map(Value::Number)
            .map(UniqueValue)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(UniqueValue(Value::String(value.to_owned())))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(UniqueValue(Value::String(value)))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(UniqueValue(Value::Null))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(UniqueValue(Value::Null))
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        UniqueValue::deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::with_capacity(sequence.size_hint().unwrap_or(0).min(1_024));
        while let Some(UniqueValue(value)) = sequence.next_element()? {
            values.push(value);
        }
        Ok(UniqueValue(Value::Array(values)))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut object = Map::new();
        let mut keys = HashSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if !keys.insert(key.clone()) {
                return Err(A::Error::custom("duplicate JSON object key"));
            }
            let UniqueValue(value) = map.next_value()?;
            object.insert(key, value);
        }
        Ok(UniqueValue(Value::Object(object)))
    }
}

fn frame_too_large() -> ClaudeError {
    ClaudeError::new(
        ClaudeErrorKind::FrameTooLarge,
        "Claude Code frame exceeds the configured limit",
    )
}

fn invalid_frame() -> ClaudeError {
    ClaudeError::new(
        ClaudeErrorKind::InvalidFrame,
        "Claude Code frame is invalid",
    )
}

fn transport_closed() -> ClaudeError {
    ClaudeError::new(
        ClaudeErrorKind::TransportClosed,
        "Claude Code transport is unavailable",
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tokio::io::AsyncWriteExt as _;

    use crate::{ClaudeErrorKind, MAX_FRAME_BYTES};

    use super::{FrameReader, decode_frame, encode_frame};

    #[test]
    fn compact_json_round_trips() {
        let frame = encode_frame(&json!({"type": "system"})).expect("frame");
        assert_eq!(
            decode_frame(&frame).expect("decoded"),
            json!({"type": "system"})
        );
    }

    #[test]
    fn duplicate_keys_invalid_utf8_and_empty_frames_are_rejected() {
        for frame in [
            &br#"{"outer":{"same":1,"same":2}}"#[..],
            &b""[..],
            &[0xff][..],
            b"   ",
        ] {
            assert_eq!(
                decode_frame(frame).expect_err("invalid frame").kind(),
                ClaudeErrorKind::InvalidFrame
            );
        }
    }

    #[test]
    fn exact_frame_limit_is_accepted_and_one_extra_byte_is_rejected() {
        let content = "a".repeat(MAX_FRAME_BYTES - 8);
        let exact = format!(r#"{{"x":"{content}"}}"#);
        assert_eq!(exact.len(), MAX_FRAME_BYTES);
        assert!(decode_frame(exact.as_bytes()).is_ok());
        assert_eq!(
            decode_frame(format!("{exact} ").as_bytes())
                .expect_err("oversized frame")
                .kind(),
            ClaudeErrorKind::FrameTooLarge
        );
    }

    #[tokio::test]
    async fn fragmented_frame_preserves_the_following_frame() {
        let mut frames = br#"{"type":"system"}"#.to_vec();
        frames.extend_from_slice(b"\n{\"type\":\"result\"}\n");
        let (mut writer, reader) = tokio::io::duplex(4);
        let writer_task = tokio::spawn(async move {
            for chunk in frames.chunks(3) {
                writer.write_all(chunk).await.expect("fragment write");
            }
        });
        let mut reader = FrameReader::new(reader);
        assert_eq!(
            reader.next_frame().await.expect("first").expect("value"),
            json!({"type": "system"})
        );
        assert_eq!(
            reader.next_frame().await.expect("second").expect("value"),
            json!({"type": "result"})
        );
        writer_task.await.expect("writer");
    }
}
