use std::{io::Cursor, time::Duration};

use aws_sdk_s3::{Client, presigning::PresigningConfig};
use symphonia::core::{
    errors::Error as SymphoniaError,
    formats::{FormatOptions, TrackType, probe::Hint},
    io::MediaSourceStream,
    meta::MetadataOptions,
    units::{Duration as SymphoniaDuration, Time as SymphoniaTime},
};

use crate::{
    config::object_storage::ObjectStorageConfig,
    domain::media::{AUDIO_CONTENT_TYPES, InspectedObject, MAX_AUDIO_BYTES, MediaKind},
    ports::object_storage::{
        InspectObjectRequest, MediaObjectStorage, MediaObjectStorageFuture,
        ObjectStorageProviderError, PresignGetRequest, PresignPutRequest, PresignedGet,
        PresignedPut,
    },
};

use super::s3_bucket_backend::{classify_provider_error, s3_client};

/// AWS S3-compatible private-object operations with separate public and internal origins.
#[derive(Clone)]
pub struct S3MediaObjectStorage {
    presign_client: Client,
    object_client: Client,
    bucket: String,
}

impl S3MediaObjectStorage {
    pub fn new(config: &ObjectStorageConfig) -> Self {
        Self {
            presign_client: s3_client(config, config.public_endpoint()),
            object_client: s3_client(config, config.endpoint()),
            bucket: config.bucket().to_owned(),
        }
    }
}

impl MediaObjectStorage for S3MediaObjectStorage {
    fn presign_put<'a>(
        &'a self,
        request: &'a PresignPutRequest,
    ) -> MediaObjectStorageFuture<'a, PresignedPut> {
        Box::pin(async move {
            let content_length = i64::try_from(request.byte_size)
                .map_err(|_| ObjectStorageProviderError::UnexpectedResponse)?;
            let presigning = PresigningConfig::expires_in(request.expires_in)
                .map_err(|_| ObjectStorageProviderError::UnexpectedResponse)?;
            let presigned = self
                .presign_client
                .put_object()
                .bucket(&self.bucket)
                .key(&request.object_key)
                .content_length(content_length)
                .content_type(&request.content_type)
                .presigned(presigning)
                .await
                .map_err(|error| classify_provider_error(&error))?;

            Ok(PresignedPut {
                url: presigned.uri().to_owned(),
                expires_in: request.expires_in,
            })
        })
    }

    fn inspect_object<'a>(
        &'a self,
        request: &'a InspectObjectRequest,
    ) -> MediaObjectStorageFuture<'a, InspectedObject> {
        Box::pin(async move {
            let head = self
                .object_client
                .head_object()
                .bucket(&self.bucket)
                .key(&request.object_key)
                .send()
                .await
                .map_err(|error| classify_provider_error(&error))?;
            let content_type = head.content_type().map(ToOwned::to_owned);
            let byte_size = head
                .content_length()
                .map(|value| {
                    u64::try_from(value).map_err(|_| ObjectStorageProviderError::UnexpectedResponse)
                })
                .transpose()?;

            let should_inspect_audio = request.kind == MediaKind::Audio
                && content_type
                    .as_deref()
                    .is_some_and(|value| AUDIO_CONTENT_TYPES.contains(&value))
                && byte_size.is_some_and(|value| value > 0 && value <= MAX_AUDIO_BYTES);
            if !should_inspect_audio {
                return Ok(InspectedObject {
                    content_type,
                    byte_size,
                    audio_duration: None,
                });
            }
            let expected_size = byte_size.ok_or(ObjectStorageProviderError::UnexpectedResponse)?;

            let object = self
                .object_client
                .get_object()
                .bucket(&self.bucket)
                .key(&request.object_key)
                .send()
                .await
                .map_err(|error| classify_provider_error(&error))?;
            let capacity = usize::try_from(expected_size)
                .map_err(|_| ObjectStorageProviderError::UnexpectedResponse)?;
            let mut stream = object.body;
            let mut body = Vec::with_capacity(capacity);
            while let Some(chunk) = stream
                .try_next()
                .await
                .map_err(|_| ObjectStorageProviderError::Unavailable)?
            {
                let next_size = body
                    .len()
                    .checked_add(chunk.len())
                    .ok_or(ObjectStorageProviderError::UnexpectedResponse)?;
                if next_size > capacity {
                    return Err(ObjectStorageProviderError::UnexpectedResponse);
                }
                body.extend_from_slice(&chunk);
            }
            let actual_size = u64::try_from(body.len())
                .map_err(|_| ObjectStorageProviderError::UnexpectedResponse)?;
            if actual_size != expected_size {
                return Err(ObjectStorageProviderError::UnexpectedResponse);
            }
            let audio_duration = inspect_audio_duration(
                body,
                content_type
                    .as_deref()
                    .ok_or(ObjectStorageProviderError::UnexpectedResponse)?,
            )?;

            Ok(InspectedObject {
                content_type,
                byte_size,
                audio_duration: Some(audio_duration),
            })
        })
    }

    fn presign_get<'a>(
        &'a self,
        request: &'a PresignGetRequest,
    ) -> MediaObjectStorageFuture<'a, PresignedGet> {
        Box::pin(async move {
            let presigning = PresigningConfig::expires_in(request.expires_in)
                .map_err(|_| ObjectStorageProviderError::UnexpectedResponse)?;
            let presigned = self
                .presign_client
                .get_object()
                .bucket(&self.bucket)
                .key(&request.object_key)
                .set_response_content_disposition(request.response_content_disposition.clone())
                .presigned(presigning)
                .await
                .map_err(|error| classify_provider_error(&error))?;

            Ok(PresignedGet {
                url: presigned.uri().to_owned(),
                expires_in: request.expires_in,
            })
        })
    }
}

fn inspect_audio_duration(
    body: Vec<u8>,
    content_type: &str,
) -> Result<Duration, ObjectStorageProviderError> {
    let source = MediaSourceStream::new(Box::new(Cursor::new(body)), Default::default());
    let mut hint = Hint::new();
    hint.mime_type(content_type);
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            source,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|_| ObjectStorageProviderError::UnexpectedResponse)?;

    let media_info = *format.media_info();
    if let (Some(time_base), Some(duration)) = (media_info.time_base, media_info.duration)
        && !duration.is_zero()
        && let Some(duration) = time_base
            .calc_duration(duration)
            .and_then(standard_duration)
    {
        return Ok(duration);
    }

    let (track_id, time_base) = format
        .default_track(TrackType::Audio)
        .and_then(|track| track.time_base.map(|time_base| (track.id, time_base)))
        .ok_or(ObjectStorageProviderError::UnexpectedResponse)?;
    let mut duration = SymphoniaDuration::ZERO;
    loop {
        match format.next_packet() {
            Ok(Some(packet)) if packet.track_id == track_id => {
                duration = duration
                    .checked_add(packet.dur)
                    .ok_or(ObjectStorageProviderError::UnexpectedResponse)?;
            }
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(SymphoniaError::IoError(error))
                if error.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(_) => return Err(ObjectStorageProviderError::UnexpectedResponse),
        }
    }
    if duration.is_zero() {
        return Err(ObjectStorageProviderError::UnexpectedResponse);
    }
    time_base
        .calc_duration(duration)
        .and_then(standard_duration)
        .ok_or(ObjectStorageProviderError::UnexpectedResponse)
}

fn standard_duration(time: SymphoniaTime) -> Option<Duration> {
    let (seconds, nanoseconds) = time.parts();
    Some(Duration::new(u64::try_from(seconds).ok()?, nanoseconds))
}
