//! The speech models Vox can use on this Mac, and how each fits it, with the
//! fit rule applied within each family.

use std::sync::OnceLock;

use crate::config::{Family, LocalModel};

pub struct Spec {
    pub model: LocalModel,
    /// The name inside its family: "Large v3 Turbo (quantised)", "4-bit".
    pub variant: &'static str,
    /// What it is good for, shown above its memory use.
    pub about: &'static str,
    /// The weights' size on Hugging Face.
    pub bytes: u64,
    /// Roughly what a run holds in memory. Whisper's come from whisper.cpp's
    /// README table. Cohere's are the peak footprint of the runtime's process
    /// transcribing a clip, measured on an Apple silicon Mac, rounded up.
    pub ram_bytes: u64,
}

impl Spec {
    /// "Whisper Large v3 Turbo (quantised)", for a sentence.
    pub fn name(&self) -> String {
        let family = match self.model.family() {
            Family::Whisper => "Whisper",
            Family::Cohere => "Cohere Transcribe",
        };
        format!("{family} {}", self.variant)
    }
}

/// Worst to best within a family.
pub const CATALOGUE: [Spec; 10] = [
    Spec {
        model: LocalModel::WhisperTiny,
        variant: "Tiny",
        about: "Lowest accuracy, fastest",
        bytes: 77_691_713,
        ram_bytes: 273_000_000,
    },
    Spec {
        model: LocalModel::WhisperBase,
        variant: "Base",
        about: "Basic accuracy, very fast",
        bytes: 147_951_465,
        ram_bytes: 388_000_000,
    },
    Spec {
        model: LocalModel::WhisperSmall,
        variant: "Small",
        about: "Fair accuracy, fast",
        bytes: 487_601_967,
        ram_bytes: 852_000_000,
    },
    Spec {
        model: LocalModel::WhisperMedium,
        variant: "Medium",
        about: "Good accuracy, slower",
        bytes: 1_533_763_059,
        ram_bytes: 2_100_000_000,
    },
    Spec {
        model: LocalModel::WhisperLargeV3TurboQ5,
        variant: "Large v3 Turbo (quantised)",
        about: "Fast, near Large v3 accuracy",
        bytes: 574_041_195,
        ram_bytes: 1_000_000_000,
    },
    Spec {
        model: LocalModel::WhisperLargeV3Turbo,
        variant: "Large v3 Turbo",
        about: "Full precision",
        bytes: 1_624_555_275,
        ram_bytes: 2_000_000_000,
    },
    Spec {
        model: LocalModel::WhisperLargeV3,
        variant: "Large v3",
        about: "Most accurate, slowest",
        bytes: 3_095_033_483,
        ram_bytes: 3_900_000_000,
    },
    Spec {
        model: LocalModel::Cohere4bit,
        variant: "4-bit",
        about: "14 languages, Auto uses English",
        bytes: 1_507_279_570,
        ram_bytes: 2_200_000_000,
    },
    Spec {
        model: LocalModel::Cohere8bit,
        variant: "8-bit",
        about: "Higher precision than 4-bit",
        bytes: 2_420_195_133,
        ram_bytes: 3_100_000_000,
    },
    Spec {
        model: LocalModel::CohereFp16,
        variant: "Full precision",
        about: "Unquantised, most memory",
        bytes: 4_131_788_382,
        ram_bytes: 4_800_000_000,
    },
];

pub fn spec(model: LocalModel) -> &'static Spec {
    CATALOGUE.iter().find(|spec| spec.model == model).expect("every model is in the catalogue")
}

/// A family's variants, worst to best.
pub fn family(family: Family) -> impl Iterator<Item = &'static Spec> {
    CATALOGUE.iter().filter(move |spec| spec.model.family() == family)
}

/// What a family recommends to a machine with room for it.
fn default_of(family: Family) -> LocalModel {
    match family {
        Family::Whisper => LocalModel::WhisperLargeV3TurboQ5,
        Family::Cohere => LocalModel::Cohere4bit,
    }
}

/// The largest variant recommended to a machine without a GPU, where the
/// large ones decode slowly.
fn small_on_cpu(family: Family) -> LocalModel {
    match family {
        Family::Whisper => LocalModel::WhisperSmall,
        Family::Cohere => LocalModel::Cohere4bit,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    Recommended,
    Ok,
    TooLarge,
}

/// How each variant of `family` suits a machine with `total` bytes of RAM.
/// Too large: it needs more than half the RAM. Recommended, exactly one: the
/// family's default if it needs at most a quarter and a GPU runs it; else the
/// largest needing at most a quarter (no larger than [`small_on_cpu`] without
/// a GPU); else the smallest. Every other variant is ok.
pub fn fits(family: Family, total: u64, gpu: bool) -> Vec<(&'static Spec, Fit)> {
    let specs: Vec<_> = self::family(family).collect();
    let index = |model| specs.iter().position(|spec| spec.model == model).unwrap_or(0);
    let comfortable = |spec: &Spec| spec.ram_bytes <= total / 4;
    let (default, cpu_cap) = (index(default_of(family)), index(small_on_cpu(family)));
    let recommended = if gpu && comfortable(specs[default]) {
        default
    } else {
        (0..specs.len())
            .rfind(|&i| comfortable(specs[i]) && (gpu || i <= cpu_cap))
            .unwrap_or(0)
    };
    specs
        .iter()
        .enumerate()
        .map(|(i, spec)| {
            let fit = match () {
                _ if i == recommended => Fit::Recommended,
                _ if spec.ram_bytes > total / 2 => Fit::TooLarge,
                _ => Fit::Ok,
            };
            (*spec, fit)
        })
        .collect()
}

/// What the fit is judged against when the RAM size cannot be read.
pub const ASSUMED_RAM: u64 = 8 << 30;

/// Physical RAM, from `hw.memsize`.
pub fn total_memory() -> Option<u64> {
    static MEMORY: OnceLock<Option<u64>> = OnceLock::new();
    *MEMORY.get_or_init(|| {
        let mut bytes: u64 = 0;
        let mut len = size_of::<u64>();
        let rc = unsafe {
            libc::sysctlbyname(
                c"hw.memsize".as_ptr(),
                (&raw mut bytes).cast(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        (rc == 0 && bytes > 0).then_some(bytes)
    })
}

/// whisper.cpp and MLX run on the GPU through Metal on Apple silicon only.
pub const GPU: bool = cfg!(target_arch = "aarch64");

/// The fit of every variant of `family` to this Mac.
pub fn fits_here(family: Family) -> Vec<(&'static Spec, Fit)> {
    fits(family, total_memory().unwrap_or(ASSUMED_RAM), GPU)
}

/// "This Mac · 16 GB memory · Apple silicon GPU". Memory in binary units, as
/// a Mac's is sold.
pub fn machine_line() -> String {
    let memory = total_memory()
        .map_or("memory unknown".into(), |bytes| format!("{} GB memory", (bytes as f64 / (1u64 << 30) as f64).round()));
    format!("This Mac · {memory} · {}", if GPU { "Apple silicon GPU" } else { "CPU only" })
}

/// A download's size, decimal as Hugging Face quotes it: "1.6 GB", "574 MB".
pub fn download_size(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1e9)
    } else {
        format!("{} MB", (bytes as f64 / 1e6).round())
    }
}

/// Memory in use, without a needless ".0": "1 GB", "3.9 GB", "388 MB".
pub fn memory_size(bytes: u64) -> String {
    download_size(bytes).replace(".0 GB", " GB")
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1 << 30;

    fn pick(family: Family, ram_gb: u64, gpu: bool) -> (LocalModel, Vec<(LocalModel, Fit)>) {
        let fits: Vec<_> =
            fits(family, ram_gb * GB, gpu).into_iter().map(|(spec, fit)| (spec.model, fit)).collect();
        let recommended: Vec<_> = fits.iter().filter(|(_, fit)| *fit == Fit::Recommended).collect();
        assert_eq!(recommended.len(), 1, "exactly one recommendation: {fits:?}");
        (recommended[0].0, fits)
    }

    fn fit_of(fits: &[(LocalModel, Fit)], model: LocalModel) -> Fit {
        fits.iter().find(|(m, _)| *m == model).unwrap().1
    }

    #[test]
    fn whisper_recommends_the_quantised_turbo_when_a_quarter_of_memory_holds_it() {
        for ram in [4, 8, 16, 64] {
            assert_eq!(pick(Family::Whisper, ram, true).0, LocalModel::WhisperLargeV3TurboQ5);
        }
    }

    #[test]
    fn whisper_drops_to_the_largest_that_fits_on_a_small_machine() {
        // A quarter of 2 GB is 0.54 GB: Base fits, Small does not.
        assert_eq!(pick(Family::Whisper, 2, true).0, LocalModel::WhisperBase);
        // Nothing fits a quarter of 1 GB: the smallest is recommended anyway.
        assert_eq!(pick(Family::Whisper, 1, true).0, LocalModel::WhisperTiny);
    }

    #[test]
    fn whisper_without_a_gpu_stops_at_small() {
        // The default needs a GPU; the largest comfortable model is capped at Small.
        assert_eq!(pick(Family::Whisper, 64, false).0, LocalModel::WhisperSmall);
    }

    #[test]
    fn over_half_the_memory_is_too_large_but_a_quarter_to_a_half_is_ok() {
        let (_, fits) = pick(Family::Whisper, 4, true);
        // Half of 4 GB is 2.15 GB.
        assert_eq!(fit_of(&fits, LocalModel::WhisperLargeV3), Fit::TooLarge);
        assert_eq!(fit_of(&fits, LocalModel::WhisperMedium), Fit::Ok);
        assert_eq!(fit_of(&fits, LocalModel::WhisperLargeV3Turbo), Fit::Ok);
        let (_, fits) = pick(Family::Whisper, 16, true);
        assert!(fits.iter().all(|(_, fit)| *fit != Fit::TooLarge));
    }

    #[test]
    fn cohere_recommends_four_bit_and_dims_what_needs_over_half_the_memory() {
        let (recommended, fits) = pick(Family::Cohere, 16, true);
        assert_eq!(recommended, LocalModel::Cohere4bit);
        assert_eq!(fit_of(&fits, LocalModel::Cohere8bit), Fit::Ok);
        assert_eq!(fit_of(&fits, LocalModel::CohereFp16), Fit::Ok);
        // 8 GB: half is 4.3 GB, and full precision needs 4.8.
        let (recommended, fits) = pick(Family::Cohere, 8, true);
        assert_eq!(recommended, LocalModel::Cohere4bit);
        assert_eq!(fit_of(&fits, LocalModel::Cohere8bit), Fit::Ok);
        assert_eq!(fit_of(&fits, LocalModel::CohereFp16), Fit::TooLarge);
        // 4 GB: even 8-bit is over half.
        let (recommended, fits) = pick(Family::Cohere, 4, true);
        assert_eq!(recommended, LocalModel::Cohere4bit);
        assert_eq!(fit_of(&fits, LocalModel::Cohere8bit), Fit::TooLarge);
    }

    #[test]
    fn unreadable_memory_is_judged_as_eight_gigabytes() {
        let fits = fits(Family::Whisper, ASSUMED_RAM, true);
        assert_eq!(fits.iter().filter(|(_, fit)| *fit == Fit::TooLarge).count(), 0);
        assert_eq!(fits.iter().find(|(_, f)| *f == Fit::Recommended).unwrap().0.model, LocalModel::WhisperLargeV3TurboQ5);
    }

    #[test]
    fn every_model_has_exactly_one_spec_and_its_family_lists_it() {
        for spec in &CATALOGUE {
            assert_eq!(CATALOGUE.iter().filter(|other| other.model == spec.model).count(), 1);
            assert!(family(spec.model.family()).any(|s| s.model == spec.model));
        }
        assert_eq!(family(Family::Whisper).count(), 7);
        assert_eq!(family(Family::Cohere).count(), 3);
    }

    #[test]
    fn sizes_read_as_they_do_on_hugging_face() {
        assert_eq!(download_size(574_041_195), "574 MB");
        assert_eq!(download_size(1_624_555_275), "1.6 GB");
        assert_eq!(memory_size(1_000_000_000), "1 GB");
        assert_eq!(memory_size(3_900_000_000), "3.9 GB");
        assert_eq!(memory_size(388_000_000), "388 MB");
    }
}
