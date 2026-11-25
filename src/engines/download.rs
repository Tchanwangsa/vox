//! Model downloads share progress reporting and publish weights only after verification.
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write as _};
use std::path::Path;
use std::pin::pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::future::{Either, select};
use sha2::{Digest as _, Sha256};

use crate::config::{data_dir, LocalModel};

/// What a cancelled download fails with, so it is not shown as a failure.
pub const CANCELLED: &str = "Cancelled";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Model {
    /// A variant of Whisper or Cohere Transcribe.
    Speech(LocalModel),
    S1,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    Preparing,
    Downloading(u8),
    Verifying,
}

impl Progress {
    pub fn message(self, model: Model) -> String {
        let name = model.label();
        match self {
            Self::Preparing => format!("Preparing {name}…"),
            Self::Downloading(percent) => format!("Downloading {name} · {percent}%"),
            Self::Verifying => format!("Verifying {name}…"),
        }
    }
}

impl Model {
    pub fn label(self) -> String {
        match self {
            Self::Speech(model) => super::models::spec(model).name(),
            Self::S1 => "S1-mini".into(),
        }
    }

    /// Download and verify it, stopping with [`CANCELLED`] soon after `cancel` is set.
    pub fn install(
        self,
        cancel: &AtomicBool,
        mut report: impl FnMut(Progress) + Send,
    ) -> Result<(), String> {
        check_cancelled(cancel)?;
        let mut last = None;
        let mut report = |progress| {
            if last != Some(progress) {
                last = Some(progress);
                report(progress);
            }
        };
        report(Progress::Preparing);
        let result = match self {
            Self::Speech(model) if model.family() == crate::config::Family::Cohere => {
                super::cohere::install(model, cancel, report)
            }
            Self::Speech(model) => {
                let pack = [&VAD, whisper_asset(model)];
                download_pack("whisper", &pack, cancel, report)
            }
            Self::S1 => download_pack("s1-mini", &S1.iter().collect::<Vec<_>>(), cancel, report),
        };
        match result {
            Err(_) if cancel.load(Ordering::Relaxed) => Err(CANCELLED.into()),
            result => result,
        }
    }
}

pub(super) fn check_cancelled(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err(CANCELLED.into())
    } else {
        Ok(())
    }
}

pub struct Asset {
    pub file: &'static str,
    url: &'static str,
    pub bytes: u64,
    sha256: &'static str,
}

/// whisper.cpp at a pinned revision, so each file's size and hash are known.
macro_rules! ggml {
    ($file:literal, $bytes:literal, $sha256:literal) => {
        Asset {
            file: $file,
            url: concat!(
                "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/",
                $file
            ),
            bytes: $bytes,
            sha256: $sha256,
        }
    };
}

/// Silero voice-activity detection, fetched with whichever model comes first.
const VAD: Asset = Asset {
    file: super::VAD_FILE,
    url: "https://huggingface.co/ggml-org/whisper-vad/resolve/9ffd54a1e1ee413ddf265af9913beaf518d1639b/ggml-silero-v6.2.0.bin",
    bytes: 885_098,
    sha256: "2aa269b785eeb53a82983a20501ddf7c1d9c48e33ab63a41391ac6c9f7fb6987",
};

/// The ggml file of a Whisper variant.
pub fn whisper_asset(model: LocalModel) -> &'static Asset {
    const TINY: Asset = ggml!("ggml-tiny.bin", 77_691_713, "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21");
    const BASE: Asset = ggml!("ggml-base.bin", 147_951_465, "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe");
    const SMALL: Asset = ggml!("ggml-small.bin", 487_601_967, "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b");
    const MEDIUM: Asset = ggml!("ggml-medium.bin", 1_533_763_059, "6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208");
    const TURBO_Q5: Asset = ggml!("ggml-large-v3-turbo-q5_0.bin", 574_041_195, "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2");
    const TURBO: Asset = ggml!("ggml-large-v3-turbo.bin", 1_624_555_275, "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69");
    const LARGE: Asset = ggml!("ggml-large-v3.bin", 3_095_033_483, "64d182b440b98d5203c4f9bd541544d84c605196c4f7b845dfa11fb23594d1e2");
    match model {
        LocalModel::WhisperTiny => &TINY,
        LocalModel::WhisperBase => &BASE,
        LocalModel::WhisperSmall => &SMALL,
        LocalModel::WhisperMedium => &MEDIUM,
        LocalModel::WhisperLargeV3TurboQ5 => &TURBO_Q5,
        LocalModel::WhisperLargeV3Turbo => &TURBO,
        LocalModel::WhisperLargeV3 => &LARGE,
        _ => panic!("{model:?} is not a Whisper model"),
    }
}

const S1: &[Asset] = &[
    Asset {
        file: "LICENSE",
        url: "https://huggingface.co/superwhisper/s1-mini-GGUF/resolve/34add00a48a2e5d24e5a4ee5405a99620a3a240c/LICENSE",
        bytes: 12_033,
        sha256: "f715982df6ce767ae64864d74b95644e5d658aab54717dbceeb252eb3cbcb421",
    },
    Asset {
        file: "NOTICE",
        url: "https://huggingface.co/superwhisper/s1-mini-GGUF/resolve/34add00a48a2e5d24e5a4ee5405a99620a3a240c/NOTICE",
        bytes: 507,
        sha256: "6eb1e98a0d0046dd1b71da19d3b64ebccc12622e7d62e5efcad10298b4d66292",
    },
    Asset {
        file: super::S1_FILE,
        url: "https://huggingface.co/superwhisper/s1-mini-GGUF/resolve/34add00a48a2e5d24e5a4ee5405a99620a3a240c/s1-mini-q4_k_m.gguf",
        bytes: 484_219_808,
        sha256: "3b41ebe2502cbd03e811d5d16b022f5ab551eda58d62597d152f89535003c634",
    },
];

pub(super) fn lock(dir: &Path) -> Result<File, String> {
    fs::create_dir_all(dir).map_err(failure)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("setup.lock"))
        .map_err(failure)?;
    lock.try_lock().map_err(|error| {
        eprintln!("[vox] Model setup lock: {error}");
        "Model setup already running".to_string()
    })?;
    Ok(lock)
}

fn download_pack(
    folder: &str,
    assets: &[&Asset],
    cancel: &AtomicBool,
    mut report: impl FnMut(Progress),
) -> Result<(), String> {
    let dir = data_dir().join("models").join(folder);
    let _lock = lock(&dir)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(failure)?;
    let result = runtime.block_on(async {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(45))
            .redirect_policy(reqwest::redirect::Policy::limited(5))
            .build()
            .map_err(failure)?;
        let total: u64 = assets.iter().map(|asset| asset.bytes).sum();
        let mut completed = 0;
        report(Progress::Downloading(0));
        for asset in assets {
            check_cancelled(cancel)?;
            if !dir.join(asset.file).is_file() {
                receive(&client, &dir, asset, cancel, |received| {
                    if completed + received == total {
                        report(Progress::Verifying);
                    } else {
                        report(Progress::Downloading(
                            ((completed + received) * 100 / total) as u8,
                        ));
                    }
                })
                .await
                .map_err(failure)?;
            }
            completed += asset.bytes;
        }
        Ok(())
    });
    // System DNS lookups can outlive a dropped request.
    runtime.shutdown_background();
    result
}

async fn receive(
    client: &reqwest::Client,
    dir: &Path,
    asset: &Asset,
    cancel: &AtomicBool,
    mut report: impl FnMut(u64),
) -> anyhow::Result<()> {
    let temp = dir.join(format!(".{}.part", asset.file));
    let transfer = async {
        let mut response = client.get(asset.url).send().await?.error_for_status()?;
        anyhow::ensure!(!cancel.load(Ordering::Relaxed), CANCELLED);
        let mut file = File::create(&temp)?;
        let mut hash = Sha256::new();
        let mut bytes = 0;
        while let Some(chunk) = response.chunk().await? {
            anyhow::ensure!(!cancel.load(Ordering::Relaxed), CANCELLED);
            bytes += chunk.len() as u64;
            if bytes > asset.bytes {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "Oversized model").into());
            }
            file.write_all(&chunk)?;
            hash.update(&chunk);
            report(bytes);
        }
        anyhow::ensure!(
            bytes == asset.bytes && format!("{:x}", hash.finalize()) == asset.sha256,
            "Model size or checksum mismatch for {}",
            asset.file
        );
        anyhow::ensure!(!cancel.load(Ordering::Relaxed), CANCELLED);
        fs::rename(&temp, dir.join(asset.file))?;
        Ok(())
    };
    let stopping = async {
        while !cancel.load(Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    let received = match select(pin!(transfer), pin!(stopping)).await {
        Either::Left((result, _)) => result,
        Either::Right(_) => Err(anyhow::anyhow!(CANCELLED)),
    };
    if received.is_err() {
        fs::remove_file(temp).ok();
    }
    received
}

fn failure(error: impl std::fmt::Display) -> String {
    eprintln!("[vox] Model download: {error}");
    "Download failed. Try again.".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Family;
    use crate::engines::models;

    #[test]
    fn cancelling_a_stalled_download_cleans_up_and_allows_retry() {
        use futures::{channel::oneshot, future::join};
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

        let dir = std::env::temp_dir().join(format!("vox-download-test-{}", std::process::id()));
        fs::create_dir(&dir).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let result = runtime.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/sample", listener.local_addr().unwrap());
            let asset = Asset {
                file: "sample",
                url: url.leak(),
                bytes: 2,
                sha256: "2689367b205c16ce32ed4200942b8b8b1e262dfc70d9bc9fbc77c49699a4f1df",
            };
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                socket.read(&mut [0; 1024]).await.unwrap();
                socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\no").await.unwrap();
                let (mut retry, _) = listener.accept().await.unwrap();
                retry.read(&mut [0; 1024]).await.unwrap();
                retry.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok").await.unwrap();
            });
            let client = reqwest::Client::builder().no_proxy().build().unwrap();
            let cancel = AtomicBool::new(false);
            let (started, partial) = oneshot::channel();
            let mut started = Some(started);
            let download = receive(&client, &dir, &asset, &cancel, |_| {
                started.take().unwrap().send(()).unwrap();
            });
            let stop = async {
                partial.await.unwrap();
                tokio::time::sleep(Duration::from_millis(100)).await;
                cancel.store(true, Ordering::Relaxed);
            };
            let result = tokio::time::timeout(Duration::from_secs(3), async {
                let (download, ()) = join(download, stop).await;
                assert_eq!(download.unwrap_err().to_string(), CANCELLED);
                assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
                cancel.store(false, Ordering::Relaxed);
                receive(&client, &dir, &asset, &cancel, |_| {}).await.unwrap();
                assert_eq!(fs::read(dir.join(asset.file)).unwrap(), b"ok");
                server.await.unwrap();
            }).await;
            result
        });
        fs::remove_dir_all(&dir).unwrap();
        result.expect("cancellation and retry should finish while the first response is stalled");
    }

    #[test]
    fn each_whisper_variants_download_is_the_size_the_menu_shows() {
        for spec in models::family(Family::Whisper) {
            let asset = whisper_asset(spec.model);
            assert_eq!(asset.bytes, spec.bytes, "{}", spec.variant);
            assert!(asset.url.ends_with(asset.file));
            assert_eq!(asset.sha256.len(), 64);
        }
    }
}
