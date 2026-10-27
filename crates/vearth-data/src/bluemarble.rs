//! NASA Blue Marble Next Generation global surface color at ~7 km/px
//! (public domain). One 5400×2700 equirectangular JPEG (~5 MB), downloaded
//! once into the cache, decoded to RGB in memory (~44 MB), sampled bilinearly
//! by latitude and longitude.
//!
//! Attribution: NASA Earth Observatory / Visible Earth, Blue Marble NG.

use crate::{ColorSource, DataError, TileFetcher};
use std::path::PathBuf;
use std::sync::Arc;

pub const DEFAULT_URL: &str = "https://eoimages.gsfc.nasa.gov/images/imagerecords/73000/73776/world.topo.bathy.200408.3x5400x2700.jpg";

pub struct BlueMarble {
    width: usize,
    height: usize,
    rgb: Vec<u8>,
}

impl BlueMarble {
    pub fn load(
        fetcher: Arc<dyn TileFetcher>,
        url: &str,
        cache_dir: Option<PathBuf>,
    ) -> Result<Self, DataError> {
        let disk_metrics = crate::metrics::acquisition("provider.bluemarble.disk");
        let bytes = if let Some(dir) = &cache_dir {
            let path = dir.join("bluemarble.jpg");
            match std::fs::read(&path) {
                Ok(b) => {
                    disk_metrics.cache_hit(b.len());
                    b
                }
                Err(_) => {
                    disk_metrics.cache_miss();
                    let b = fetcher.fetch(url)?;
                    let _ = std::fs::create_dir_all(dir);
                    let _ = std::fs::write(&path, &b);
                    b
                }
            }
        } else {
            fetcher.fetch(url)?
        };
        Self::decode(&bytes)
    }

    #[cfg(feature = "async-http")]
    pub async fn load_async(
        fetcher: Arc<dyn crate::async_http::AsyncTileFetcher>,
        url: &str,
        cache_dir: Option<PathBuf>,
    ) -> Result<Self, DataError> {
        let disk_metrics = crate::metrics::acquisition("provider.bluemarble.disk");
        let path = cache_dir
            .as_ref()
            .map(|directory| directory.join("bluemarble.jpg"));
        let (bytes, persist_path) = if let Some(path) = path {
            match vearth_platform::rt::task::spawn_blocking(move || std::fs::read(path)).await {
                Ok(Ok(bytes)) => {
                    disk_metrics.cache_hit(bytes.len());
                    (bytes, None)
                }
                _ => {
                    disk_metrics.cache_miss();
                    let bytes = fetcher.fetch(url).await?;
                    (
                        bytes,
                        cache_dir.map(|directory| directory.join("bluemarble.jpg")),
                    )
                }
            }
        } else {
            (fetcher.fetch(url).await?, None)
        };
        let (image, bytes) = vearth_platform::rt::task::spawn_blocking(move || {
            let image = Self::decode(&bytes)?;
            Ok::<_, DataError>((image, bytes))
        })
        .await
        .map_err(|error| DataError::Decode(format!("Blue Marble decode task: {error}")))??;
        if let Some(path) = persist_path {
            let _ = vearth_platform::rt::task::spawn_blocking(move || {
                crate::async_prep::write_atomic(&path, &bytes)
            })
            .await;
        }
        Ok(image)
    }

    fn decode(bytes: &[u8]) -> Result<Self, DataError> {
        let (width, height, rgb) = crate::jpeg::decode_rgb(bytes)?;
        Ok(BlueMarble { width, height, rgb })
    }
}

impl ColorSource for BlueMarble {
    fn color(&self, lat: f64, lon: f64) -> Option<[u8; 3]> {
        // Equirectangular: x from lon, y from lat (north at row 0).
        let fx = (lon.to_degrees() + 180.0) / 360.0 * self.width as f64 - 0.5;
        let fy = (90.0 - lat.to_degrees()) / 180.0 * self.height as f64 - 0.5;
        let x0 = fx.floor() as i64;
        let y0 = fy.floor() as i64;
        let (tx, ty) = ((fx - x0 as f64) as f32, (fy - y0 as f64) as f32);
        let px = |x: i64, y: i64, c: usize| -> f32 {
            let x = x.rem_euclid(self.width as i64) as usize;
            let y = y.clamp(0, self.height as i64 - 1) as usize;
            self.rgb[(y * self.width + x) * 3 + c] as f32
        };
        let mut out = [0u8; 3];
        for (c, o) in out.iter_mut().enumerate() {
            let a = px(x0, y0, c) + (px(x0 + 1, y0, c) - px(x0, y0, c)) * tx;
            let b = px(x0, y0 + 1, c) + (px(x0 + 1, y0 + 1, c) - px(x0, y0 + 1, c)) * tx;
            *o = (a + (b - a) * ty).round().clamp(0.0, 255.0) as u8;
        }
        Some(out)
    }
}

#[cfg(all(test, feature = "async-http"))]
mod async_tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct CountingFetcher {
        bytes: Vec<u8>,
        calls: AtomicU32,
    }

    impl crate::async_http::AsyncTileFetcher for CountingFetcher {
        fn fetch<'a>(&'a self, _url: &'a str) -> crate::async_http::FetchFuture<'a> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let bytes = self.bytes.clone();
            Box::pin(async move { Ok(bytes) })
        }
    }

    fn runtime() -> vearth_platform::rt::runtime::Runtime {
        vearth_platform::rt::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
    }

    fn temp_dir(label: &str) -> PathBuf {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        std::env::temp_dir().join(format!(
            "vearth-bluemarble-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn async_disk_hit_never_starts_network_even_when_decode_fails() {
        let dir = temp_dir("hit");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("bluemarble.jpg"), b"bad cached jpeg").unwrap();
        let fetcher = Arc::new(CountingFetcher {
            bytes: b"network bytes".to_vec(),
            calls: AtomicU32::new(0),
        });
        let result = runtime().block_on(BlueMarble::load_async(
            fetcher.clone(),
            "http://unused",
            Some(dir.clone()),
        ));
        assert!(matches!(result, Err(DataError::Decode(_))));
        assert_eq!(fetcher.calls.load(Ordering::Relaxed), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn async_cache_miss_fetches_once_and_does_not_persist_bad_bytes() {
        let dir = temp_dir("miss");
        let fetcher = Arc::new(CountingFetcher {
            bytes: b"bad network jpeg".to_vec(),
            calls: AtomicU32::new(0),
        });
        let result = runtime().block_on(BlueMarble::load_async(
            fetcher.clone(),
            "http://unused",
            Some(dir.clone()),
        ));
        assert!(matches!(result, Err(DataError::Decode(_))));
        assert_eq!(fetcher.calls.load(Ordering::Relaxed), 1);
        assert!(!dir.join("bluemarble.jpg").exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
