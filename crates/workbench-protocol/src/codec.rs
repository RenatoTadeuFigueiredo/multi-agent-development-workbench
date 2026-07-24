use std::{io, marker::PhantomData};

use bytes::{Buf, BufMut, BytesMut};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use thiserror::Error;
use tokio_util::codec::{Decoder, Encoder};

use crate::PROTOCOL_V1;
use crate::validation::StrictValue;

pub const MAX_FRAME_BYTES: usize = 8_388_608;

#[derive(Debug, Error)]
pub enum ProtocolCodecError {
    #[error("protocol frame exceeds 8 MiB")]
    FrameTooLarge,
    #[error("unsupported protocol major")]
    UnsupportedVersion,
    #[error("protocol frame is incomplete")]
    IncompleteFrame,
    #[error("invalid protocol JSON: {0}")]
    InvalidJson(String),
    #[error("protocol I/O failed: {0}")]
    Io(#[from] io::Error),
}

#[derive(Clone, Debug)]
pub struct NdjsonCodec<Inbound, Outbound> {
    _types: PhantomData<(Inbound, Outbound)>,
}

impl<Inbound, Outbound> Default for NdjsonCodec<Inbound, Outbound> {
    fn default() -> Self {
        Self {
            _types: PhantomData,
        }
    }
}

impl<Inbound, Outbound> Decoder for NdjsonCodec<Inbound, Outbound>
where
    Inbound: DeserializeOwned,
{
    type Item = Inbound;
    type Error = ProtocolCodecError;

    fn decode(&mut self, source: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let Some(newline) = source.iter().position(|byte| *byte == b'\n') else {
            if source.len() > MAX_FRAME_BYTES {
                source.clear();
                return Err(ProtocolCodecError::FrameTooLarge);
            }
            return Ok(None);
        };
        if newline > MAX_FRAME_BYTES {
            source.advance(newline + 1);
            return Err(ProtocolCodecError::FrameTooLarge);
        }
        let frame = source.split_to(newline);
        source.advance(1);
        decode_frame(&frame).map(Some)
    }

    fn decode_eof(&mut self, source: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if source.is_empty() {
            Ok(None)
        } else if source.len() > MAX_FRAME_BYTES {
            source.clear();
            Err(ProtocolCodecError::FrameTooLarge)
        } else {
            source.clear();
            Err(ProtocolCodecError::IncompleteFrame)
        }
    }
}

impl<Inbound, Outbound> Encoder<Outbound> for NdjsonCodec<Inbound, Outbound>
where
    Outbound: Serialize,
{
    type Error = ProtocolCodecError;

    fn encode(&mut self, item: Outbound, destination: &mut BytesMut) -> Result<(), Self::Error> {
        let encoded = serde_json::to_vec(&item)
            .map_err(|error| ProtocolCodecError::InvalidJson(error.to_string()))?;
        if encoded.len() > MAX_FRAME_BYTES {
            return Err(ProtocolCodecError::FrameTooLarge);
        }
        destination.reserve(encoded.len() + 1);
        destination.put_slice(&encoded);
        destination.put_u8(b'\n');
        Ok(())
    }
}

fn decode_frame<T: DeserializeOwned>(frame: &[u8]) -> Result<T, ProtocolCodecError> {
    if frame.is_empty() {
        return Err(ProtocolCodecError::InvalidJson(
            "empty protocol frame".to_owned(),
        ));
    }
    let mut deserializer = serde_json::Deserializer::from_slice(frame);
    let strict = StrictValue::deserialize(&mut deserializer)
        .map_err(|error| ProtocolCodecError::InvalidJson(error.to_string()))?;
    deserializer
        .end()
        .map_err(|error| ProtocolCodecError::InvalidJson(error.to_string()))?;
    if strict
        .0
        .as_object()
        .and_then(|object| object.get("protocol"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|protocol| protocol != PROTOCOL_V1)
    {
        return Err(ProtocolCodecError::UnsupportedVersion);
    }
    serde_json::from_value(strict.0)
        .map_err(|error| ProtocolCodecError::InvalidJson(error.to_string()))
}
