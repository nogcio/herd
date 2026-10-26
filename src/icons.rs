//! Bounded, validated raster artwork with an on-disk cache. Independent of COSMIC.
use anyhow::{Context, Result, anyhow, ensure};
use sha2::{Digest, Sha256};
use std::{
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use url::Url;

const MAX_BYTES: usize = 512 * 1024;
const ICON_SIZE: u32 = 128;
const HOSTS: &[&str] = &[
    "huggingface.co",
    "cdn-avatars.huggingface.co",
    "raw.githubusercontent.com",
];

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
/// Untrusted optional artwork references supplied by native preset comments.
pub struct IconSpec {
    /// An absolute raster path or a desktop theme icon name (resolved by the UI).
    pub local: Option<String>,
    /// Optional HTTPS source; validated at fetch time, never inferred from a model name.
    pub url: Option<String>,
}

impl IconSpec {
    /// Whether no artwork source was provided.
    pub fn is_empty(&self) -> bool {
        self.local.is_none() && self.url.is_none()
    }
    /// A syntactically valid theme icon name, without resolving it or touching disk.
    pub fn theme_name(&self) -> Option<&str> {
        self.local.as_deref().filter(|value| {
            !value.is_empty()
                && value
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// How a successfully decoded image was obtained.
pub enum Origin {
    /// A user-specified local raster file.
    Local,
    /// A previously validated on-disk thumbnail.
    Cached,
    /// A new, validated HTTPS response, regardless of cache-write success.
    Downloaded,
}

#[derive(Clone, Debug)]
/// Normalized raster bytes ready for presentation; no network/file access is needed.
pub struct LoadedIcon {
    /// PNG thumbnail fitting within 128×128 pixels, with aspect ratio preserved.
    pub png: Vec<u8>,
    /// Source used by the loader.
    pub origin: Origin,
}

/// Restricted HTTPS downloader, bounded raster decoder and atomic disk cache.
pub struct IconLoader {
    cache: PathBuf,
    http: reqwest::Client,
    requests: tokio::sync::Semaphore,
}

impl IconLoader {
    /// Configure the XDG cache/client without fetching images or creating directories.
    /// Fails if the cache location or HTTP client cannot be configured.
    pub fn new() -> Result<Self> {
        Self::with_cache(
            dirs::cache_dir()
                .context("Cannot locate the cache directory")?
                .join("herd/icons"),
        )
    }

    fn with_cache(cache: PathBuf) -> Result<Self> {
        ensure!(cache.is_absolute(), "Icon cache path must be absolute");
        let http = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 5 || validate_url(attempt.url()).is_err() {
                    attempt.error("Icon redirect is not allowed")
                } else {
                    attempt.follow()
                }
            }))
            .build()?;
        Ok(Self {
            cache,
            http,
            requests: tokio::sync::Semaphore::new(4),
        })
    }

    /// The caller should resolve theme names before this raster/local/cache loader.
    ///
    /// Local/cache decoding and writes run on blocking workers. Errors leave
    /// service policy untouched; callers should retain an initials fallback.
    /// Only approved HTTPS hosts, bounded formats and bounded redirects are admitted.
    pub async fn load(self: &Arc<Self>, spec: IconSpec) -> Result<LoadedIcon> {
        if let Some(path) = spec.local.as_deref().filter(|p| Path::new(p).is_absolute()) {
            let path = PathBuf::from(path);
            let local =
                tokio::task::spawn_blocking(move || normalize(&read_bounded(&path)?)).await?;
            if let Ok(png) = local {
                return Ok(LoadedIcon {
                    png,
                    origin: Origin::Local,
                });
            }
        }
        let value = spec.url.context("No usable raster icon source")?;
        let url = Url::parse(&value).map_err(|_| anyhow!("Invalid icon URL"))?;
        validate_url(&url)?;
        let cache = self.cache.join(format!(
            "{}.png",
            hex::encode(Sha256::digest(url.as_str().as_bytes()))
        ));
        let read_path = cache.clone();
        if let Ok(png) =
            tokio::task::spawn_blocking(move || normalize(&read_bounded(&read_path)?)).await?
        {
            return Ok(LoadedIcon {
                png,
                origin: Origin::Cached,
            });
        }
        let _permit = self.requests.acquire().await?;
        let mut response = self
            .http
            .get(url)
            .header(reqwest::header::ACCEPT, "image/png, image/jpeg, image/webp")
            .send()
            .await
            .map_err(|_| anyhow!("Icon request failed"))?;
        ensure!(
            response.status().is_success(),
            "Icon server returned an unsuccessful status"
        );
        let mime = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim();
        ensure!(
            [
                "image/png",
                "image/jpeg",
                "image/webp",
                "application/octet-stream"
            ]
            .contains(&mime),
            "Icon server did not return a supported image type"
        );
        ensure!(
            response
                .content_length()
                .is_none_or(|size| size <= MAX_BYTES as u64),
            "Icon exceeds 512 KiB"
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| anyhow!("Icon download failed"))?
        {
            ensure!(
                bytes.len() + chunk.len() <= MAX_BYTES,
                "Icon exceeds 512 KiB"
            );
            bytes.extend_from_slice(&chunk);
        }
        tokio::task::spawn_blocking(move || {
            let png = normalize(&bytes)?;
            // A read-only/full cache must not prevent displaying a valid downloaded icon.
            let _ = write_cache(&cache, &png);
            Ok(LoadedIcon {
                png,
                origin: Origin::Downloaded,
            })
        })
        .await?
    }
}

fn validate_url(url: &Url) -> Result<()> {
    ensure!(
        url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && url.port().is_none_or(|port| port == 443)
            && url.query().is_none()
            && url.fragment().is_none()
            && url.host_str().is_some_and(|host| HOSTS.contains(&host)),
        "Icon URL must use HTTPS on an approved artwork host, without credentials, query or fragment"
    );
    Ok(())
}

fn read_bounded(path: &Path) -> Result<Vec<u8>> {
    let file = crate::regular_file::open(path)?;
    let mut bytes = Vec::new();
    file.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= MAX_BYTES, "Icon exceeds 512 KiB");
    Ok(bytes)
}

fn normalize(bytes: &[u8]) -> Result<Vec<u8>> {
    ensure!(bytes.len() <= MAX_BYTES, "Icon exceeds 512 KiB");
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    ensure!(
        matches!(
            reader.format(),
            Some(image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::WebP)
        ),
        "Only PNG, JPEG and WebP artwork is supported"
    );
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(2048);
    limits.max_image_height = Some(2048);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|_| anyhow!("Unsupported or damaged icon image"))?;
    let thumbnail = image.resize(ICON_SIZE, ICON_SIZE, image::imageops::FilterType::Lanczos3);
    let mut png = Cursor::new(Vec::new());
    thumbnail.write_to(&mut png, image::ImageFormat::Png)?;
    Ok(png.into_inner())
}

fn write_cache(path: &Path, png: &[u8]) -> Result<()> {
    let parent = path.parent().context("Invalid icon cache path")?;
    std::fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(png)?;
    temp.persist(path)
        .map_err(|_| anyhow!("Cannot save icon cache"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn png() -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(80, 40)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    }

    #[test]
    fn artwork_reads_reject_fifos_but_accept_regular_symlink_targets() {
        use crate::regular_file::test_support;
        if let Some(root) = test_support::child_root() {
            let fifo = root.join("fifo");
            rustix::fs::mkfifoat(
                rustix::fs::CWD,
                &fifo,
                rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
            )
            .unwrap();
            assert!(read_bounded(&fifo).is_err());
            let link = root.join("link");
            std::os::unix::fs::symlink(&fifo, &link).unwrap();
            assert!(read_bounded(&link).is_err());
            assert!(read_bounded(&root).is_err());
            std::fs::remove_file(&link).unwrap();
            let image = root.join("local.png");
            let bytes = png();
            std::fs::write(&image, &bytes).unwrap();
            std::os::unix::fs::symlink(&image, &link).unwrap();
            assert_eq!(read_bounded(&link).unwrap(), bytes);
            assert!(normalize(&read_bounded(&link).unwrap()).is_ok());
            assert_eq!(std::fs::read(&image).unwrap(), bytes);
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        test_support::run(
            "icons::tests::artwork_reads_reject_fifos_but_accept_regular_symlink_targets",
            dir.path(),
        );
    }

    #[test]
    fn remote_sources_and_redirects_are_restricted() {
        for value in [
            "https://huggingface.co/org/model/resolve/main/logo.png",
            "https://cdn-avatars.huggingface.co/avatar.webp",
            "https://raw.githubusercontent.com/org/project/main/logo.png",
        ] {
            assert!(validate_url(&Url::parse(value).unwrap()).is_ok());
        }
        for value in [
            "http://huggingface.co/logo",
            "https://127.0.0.1/logo",
            "https://huggingface.co.evil.example/logo",
            "https://user:secret@huggingface.co/logo",
            "https://huggingface.co:8443/logo",
            "https://huggingface.co/logo?token=secret",
            "https://example.com/logo",
        ] {
            assert!(validate_url(&Url::parse(value).unwrap()).is_err());
        }
    }

    #[test]
    fn validates_content_and_preserves_aspect_ratio() {
        let data = normalize(&png()).unwrap();
        let image = image::load_from_memory(&data).unwrap();
        assert_eq!((image.width(), image.height()), (128, 64));
        assert!(normalize(b"<svg><script>bad</script></svg>").is_err());
        assert!(normalize(b"not an image").is_err());
        assert!(normalize(&vec![0; MAX_BYTES + 1]).is_err());
    }

    #[test]
    fn decoder_rejects_excessive_dimensions_before_resizing() {
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(2049, 1)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        assert!(normalize(&bytes.into_inner()).is_err());
    }

    #[tokio::test]
    async fn valid_cache_is_used_without_network() {
        let dir = tempfile::tempdir().unwrap();
        let loader = Arc::new(IconLoader::with_cache(dir.path().into()).unwrap());
        let url = "https://huggingface.co/test/does-not-exist.png";
        let path = dir.path().join(format!(
            "{}.png",
            hex::encode(Sha256::digest(url.as_bytes()))
        ));
        write_cache(&path, &normalize(&png()).unwrap()).unwrap();
        let loaded = loader
            .load(IconSpec {
                url: Some(url.into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(loaded.origin, Origin::Cached);
    }

    #[tokio::test]
    async fn local_artwork_takes_priority_and_bad_sources_fail_safely() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("local.png");
        std::fs::write(&path, png()).unwrap();
        let loader = Arc::new(IconLoader::with_cache(dir.path().join("cache")).unwrap());
        let loaded = loader
            .load(IconSpec {
                local: Some(path.to_string_lossy().into()),
                url: Some("invalid".into()),
            })
            .await
            .unwrap();
        assert_eq!(loaded.origin, Origin::Local);
        assert!(
            loader
                .load(IconSpec {
                    url: Some("http://localhost/secret".into()),
                    ..Default::default()
                })
                .await
                .is_err()
        );
    }
}
