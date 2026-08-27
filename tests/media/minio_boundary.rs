use std::{env, fs, io, path::Path, time::Duration};

use aws_sdk_s3::{
    Client as S3Client,
    config::{Credentials, Region},
};
use jamye_server::{
    adapters::object_storage::media::{
        BucketEnsureOutcome, BucketLifecycle, S3BucketBackend, S3MediaObjectStorage,
    },
    config::{
        AppEnvironment,
        object_storage::{ObjectStorageConfig, ObjectStorageConfigInput},
    },
    domain::media::{MediaKind, PRESIGNED_GET_TTL_SECONDS, PRESIGNED_PUT_TTL_SECONDS},
    ports::object_storage::{
        BucketLifecycleBackend, InspectObjectRequest, MediaObjectStorage,
        ObjectStorageProviderError, PresignGetRequest, PresignPutRequest,
    },
};
use reqwest::{
    Client, Response, StatusCode,
    header::{CONTENT_LENGTH, CONTENT_TYPE},
    redirect::Policy,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::TestResult;

const MINIO_ORIGIN: &str = "http://127.0.0.1:9000";
const MINIO_HEALTH_URL: &str = "http://127.0.0.1:9000/minio/health/live";
const PRIVATE_BUCKET: &str = "jamye-task8-media";
const POLICY_PATH: &str = "scripts/tasks/task-8/minio-app-policy.json";

#[test]
fn disposable_app_policy_is_exactly_scoped_to_task_8_media() -> TestResult {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(POLICY_PATH);
    let policy = fs::read_to_string(&path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            io::Error::other(format!(
                "RED: {POLICY_PATH} is absent; task-8 must publish the disposable MinIO least-privilege app policy"
            ))
        } else {
            error
        }
    })?;
    let actual: Value = serde_json::from_str(&policy)?;
    let expected = json!({
        "Version": "2012-10-17",
        "Statement": [
            {
                "Sid": "Task8BucketLifecycle",
                "Effect": "Allow",
                "Action": [
                    "s3:CreateBucket",
                    "s3:GetBucketLocation",
                    "s3:ListBucket"
                ],
                "Resource": [format!("arn:aws:s3:::{PRIVATE_BUCKET}")]
            },
            {
                "Sid": "Task8PrivateObjects",
                "Effect": "Allow",
                "Action": ["s3:GetObject", "s3:PutObject"],
                "Resource": [format!("arn:aws:s3:::{PRIVATE_BUCKET}/*")]
            }
        ]
    });

    assert_eq!(actual, expected);
    let serialized = serde_json::to_string(&actual)?;
    for forbidden in [
        "admin:",
        "s3:*",
        "s3:DeleteBucket",
        "s3:DeleteObject",
        "s3:ListAllMyBuckets",
        "arn:aws:s3:::*\"",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "disposable app policy granted {forbidden}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn app_identity_creates_only_the_selected_private_bucket_and_serves_signed_objects()
-> TestResult {
    let credentials = guarded_credentials()?;
    reset_disposable_bucket(&credentials).await?;
    let config = object_storage_config(&credentials)?;
    let debug = format!("{config:?}");
    for secret in credentials.all_values() {
        assert!(
            !debug.contains(secret),
            "object-storage Debug leaked a credential"
        );
    }
    assert!(debug.contains("[REDACTED]"));

    let backend = S3BucketBackend::new(&config);
    let lifecycle = BucketLifecycle::new(backend.clone(), config.bucket());
    assert_eq!(
        lifecycle.ensure_bucket().await,
        Ok(BucketEnsureOutcome::Created)
    );
    assert_eq!(
        backend.create_bucket("jamye-task8-forbidden").await,
        Err(ObjectStorageProviderError::AccessDenied)
    );

    let storage = S3MediaObjectStorage::new(&config);
    let object_key = format!("chat/{}/{}", Uuid::new_v4(), Uuid::new_v4());
    let payload = b"task-8-private-image";
    let put = storage
        .presign_put(&PresignPutRequest {
            object_key: object_key.clone(),
            content_type: "image/png".to_owned(),
            byte_size: u64::try_from(payload.len())?,
            expires_in: Duration::from_secs(PRESIGNED_PUT_TTL_SECONDS),
        })
        .await?;
    assert_eq!(put.expires_in.as_secs(), PRESIGNED_PUT_TTL_SECONDS);
    assert_public_material_excludes_secrets(&put.url, &credentials);

    let client = http_client()?;
    let uploaded = client
        .put(&put.url)
        .header(CONTENT_TYPE, "image/png")
        .header(CONTENT_LENGTH, payload.len().to_string())
        .body(payload.to_vec())
        .send()
        .await?;
    assert_eq!(uploaded.status(), StatusCode::OK);

    let inspected = storage
        .inspect_object(&InspectObjectRequest {
            object_key: object_key.clone(),
            kind: MediaKind::Image,
        })
        .await?;
    assert_eq!(inspected.content_type.as_deref(), Some("image/png"));
    assert_eq!(inspected.byte_size, Some(u64::try_from(payload.len())?));
    assert_eq!(inspected.audio_duration, None);

    let get = storage
        .presign_get(&PresignGetRequest {
            object_key: object_key.clone(),
            response_content_disposition: None,
            expires_in: Duration::from_secs(PRESIGNED_GET_TTL_SECONDS),
        })
        .await?;
    assert_eq!(get.expires_in.as_secs(), PRESIGNED_GET_TTL_SECONDS);
    assert_public_material_excludes_secrets(&get.url, &credentials);
    let downloaded = client.get(&get.url).send().await?;
    assert_eq!(downloaded.status(), StatusCode::OK);
    assert_eq!(downloaded.bytes().await?.as_ref(), payload);

    for response in [
        client
            .get(format!("{MINIO_ORIGIN}/{PRIVATE_BUCKET}?list-type=2"))
            .send()
            .await?,
        client
            .head(format!("{MINIO_ORIGIN}/{PRIVATE_BUCKET}/{object_key}"))
            .send()
            .await?,
        client
            .get(format!("{MINIO_ORIGIN}/{PRIVATE_BUCKET}/{object_key}"))
            .send()
            .await?,
    ] {
        assert_anonymous_denied(response, &credentials).await?;
    }
    reset_disposable_bucket(&credentials).await?;
    Ok(())
}

async fn reset_disposable_bucket(credentials: &DisposableCredentials) -> TestResult {
    let client = admin_s3_client(credentials);
    let mut continuation_token = None;

    loop {
        let listed = match client
            .list_objects_v2()
            .bucket(PRIVATE_BUCKET)
            .set_continuation_token(continuation_token)
            .send()
            .await
        {
            Ok(listed) => listed,
            Err(error)
                if error
                    .raw_response()
                    .is_some_and(|response| response.status().as_u16() == 404) =>
            {
                return Ok(());
            }
            Err(_) => {
                return Err(io::Error::other(
                    "disposable MinIO admin could not list the fixed task-8 bucket",
                )
                .into());
            }
        };

        for object in listed.contents() {
            let key = object.key().ok_or_else(|| {
                io::Error::other("disposable MinIO returned an object without a key")
            })?;
            client
                .delete_object()
                .bucket(PRIVATE_BUCKET)
                .key(key)
                .send()
                .await
                .map_err(|_| {
                    io::Error::other("disposable MinIO admin could not delete a task-8 test object")
                })?;
        }

        if listed.is_truncated() != Some(true) {
            break;
        }
        continuation_token = Some(
            listed
                .next_continuation_token()
                .ok_or_else(|| {
                    io::Error::other(
                        "disposable MinIO returned a truncated page without a continuation token",
                    )
                })?
                .to_owned(),
        );
    }

    client
        .delete_bucket()
        .bucket(PRIVATE_BUCKET)
        .send()
        .await
        .map_err(|_| {
            io::Error::other("disposable MinIO admin could not delete the fixed task-8 bucket")
        })?;
    Ok(())
}

fn admin_s3_client(credentials: &DisposableCredentials) -> S3Client {
    let credentials = Credentials::new(
        credentials.admin_user.clone(),
        credentials.admin_password.clone(),
        None,
        None,
        "task-8-disposable-admin",
    );
    let config = aws_sdk_s3::Config::builder()
        .behavior_version_latest()
        .region(Region::new("us-east-1"))
        .credentials_provider(credentials)
        .endpoint_url(MINIO_ORIGIN)
        .force_path_style(true)
        .build();

    S3Client::from_conf(config)
}

fn guarded_credentials() -> TestResult<DisposableCredentials> {
    if env::var("JAMYE_ENVIRONMENT").as_deref() != Ok("test")
        || env::var("JAMYE_MINIO_HEALTH_URL").as_deref() != Ok(MINIO_HEALTH_URL)
    {
        return Err(io::Error::other(
            "disposable MinIO integration accepts only the task-1 loopback test environment",
        )
        .into());
    }
    let credentials = DisposableCredentials {
        admin_user: required_env("JAMYE_TEST_MINIO_ADMIN_USER")?,
        admin_password: required_env("JAMYE_TEST_MINIO_ADMIN_PASSWORD")?,
        app_user: required_env("JAMYE_TEST_MINIO_APP_USER")?,
        app_password: required_env("JAMYE_TEST_MINIO_APP_PASSWORD")?,
    };
    if credentials.admin_user == credentials.app_user
        || credentials.admin_password == credentials.app_password
    {
        return Err(
            io::Error::other("MinIO admin and app credentials must remain distinct").into(),
        );
    }
    Ok(credentials)
}

fn required_env(key: &'static str) -> TestResult<String> {
    env::var(key)
        .ok()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| io::Error::other(format!("{key} is required for disposable MinIO tests")))
        .map_err(Into::into)
}

fn object_storage_config(credentials: &DisposableCredentials) -> TestResult<ObjectStorageConfig> {
    ObjectStorageConfig::resolve(
        AppEnvironment::Test,
        ObjectStorageConfigInput {
            endpoint: Some(MINIO_ORIGIN.to_owned()),
            public_endpoint: Some(MINIO_ORIGIN.to_owned()),
            region: Some("us-east-1".to_owned()),
            bucket: Some(PRIVATE_BUCKET.to_owned()),
            access_key_id: Some(credentials.app_user.clone()),
            secret_access_key: Some(credentials.app_password.clone()),
        },
    )?
    .ok_or_else(|| io::Error::other("complete disposable MinIO config resolved absent").into())
}

fn http_client() -> TestResult<Client> {
    Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(10))
        .redirect(Policy::none())
        .no_proxy()
        .build()
        .map_err(Into::into)
}

fn assert_public_material_excludes_secrets(material: &str, credentials: &DisposableCredentials) {
    for secret in [
        credentials.admin_user.as_str(),
        credentials.admin_password.as_str(),
        credentials.app_password.as_str(),
    ] {
        assert!(
            !material.contains(secret),
            "public object-storage material leaked a secret"
        );
    }
}

async fn assert_anonymous_denied(
    response: Response,
    credentials: &DisposableCredentials,
) -> TestResult {
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let headers = format!("{:?}", response.headers());
    assert_public_material_excludes_secrets(&headers, credentials);
    let body = String::from_utf8_lossy(&response.bytes().await?).into_owned();
    assert_public_material_excludes_secrets(&body, credentials);
    Ok(())
}

struct DisposableCredentials {
    admin_user: String,
    admin_password: String,
    app_user: String,
    app_password: String,
}

impl DisposableCredentials {
    fn all_values(&self) -> [&str; 4] {
        [
            &self.admin_user,
            &self.admin_password,
            &self.app_user,
            &self.app_password,
        ]
    }
}
