//! Placement of WASM sheet images in the board's `crabmod` flash partition.
//!
//! Pure logic, no I/O: decides for every sheet whether it is stored as an AOT
//! image (fast, larger) or as a plain `.wasm` run by the interpreter.

use crab_core::logic::SheetSpeed;

/// How a sheet image is stored and executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Ahead-of-time compiled image.
    Aot,
    /// Plain `.wasm` run by the interpreter.
    Interp,
}

/// Input description of one sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SheetImage {
    pub number: u32,
    pub speed: SheetSpeed,
    /// Size of the plain `.wasm` module.
    pub wasm_bytes: usize,
    /// Size of the AOT image, `None` if it could not be built (no `wamrc`).
    pub aot_bytes: Option<usize>,
    /// Sheet period in milliseconds.
    pub period_ms: u32,
    /// Estimated step time in the interpreter, microseconds.
    pub step_interp_us: u32,
    /// Estimated step time as AOT, microseconds.
    pub step_aot_us: u32,
}

/// Why a Normal sheet runs in the interpreter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemoteReason {
    /// Flash is short, the sheet was demoted to make everything fit.
    NoSpace,
    /// There is no AOT image for the sheet.
    NoAotImage,
}

/// Placement result for one sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placed {
    pub number: u32,
    pub mode: Mode,
    /// Size of the chosen image.
    pub bytes: usize,
    /// Set when a Normal sheet was moved to the interpreter.
    pub demoted: Option<DemoteReason>,
    /// Estimated step time for the chosen mode, microseconds.
    pub step_us: u32,
    /// `step_us` is longer than the sheet period.
    pub exceeds_period: bool,
}

/// Placement of all sheets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    /// In input order.
    pub sheets: Vec<Placed>,
    /// Total bytes of the chosen images.
    pub needed: usize,
    /// Free bytes that were available.
    pub free: usize,
}

impl Placement {
    /// Sheets that were moved to the interpreter (for any reason).
    pub fn demoted(&self) -> impl Iterator<Item = &Placed> {
        self.sheets.iter().filter(|p| p.demoted.is_some())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PlacementError {
    #[error("лист {number}: «Быстрый» требует ускоренной сборки, а она недоступна (нет wamrc или мастер её не поддерживает) — скачайте wamrc, перезалейте мастер или выберите «Обычный»")]
    FastWithoutAot { number: u32 },
    #[error("«Быстрые» листы не помещаются в память модулей: нужно {needed} байт, свободно {free} — удалите модули с платы или разделите лист")]
    FastDoesNotFit { needed: usize, free: usize },
    #[error("листы не помещаются в память модулей даже без ускорения: нужно {needed} байт, свободно {free} — удалите модули с платы или разделите лист")]
    DoesNotFit { needed: usize, free: usize },
}

/// Chooses a storage mode for every sheet.
///
/// `free_bytes` is computed by the caller: free space in `crabmod`, including
/// the space of images that are going to be replaced by this upload.
///
/// Background sheets always use the interpreter; Fast sheets always use AOT
/// and are never demoted. Normal sheets use AOT when an image exists; when
/// flash is short they are demoted one at a time: longest period first, then
/// lightest interpreter step, then lowest sheet number.
///
/// `FastDoesNotFit` reports the bytes of the Fast AOT images alone; `DoesNotFit`
/// reports the minimum total (Fast as AOT, everything else as `.wasm`).
pub fn place(sheets: &[SheetImage], free_bytes: usize) -> Result<Placement, PlacementError> {
    let mut fast_bytes = 0usize;
    let mut chosen: Vec<(Mode, usize, Option<DemoteReason>)> = Vec::with_capacity(sheets.len());
    for s in sheets {
        let entry = match s.speed {
            SheetSpeed::Background => (Mode::Interp, s.wasm_bytes, None),
            SheetSpeed::Fast => {
                let aot = s
                    .aot_bytes
                    .ok_or(PlacementError::FastWithoutAot { number: s.number })?;
                fast_bytes += aot;
                (Mode::Aot, aot, None)
            }
            SheetSpeed::Normal => match s.aot_bytes {
                Some(aot) => (Mode::Aot, aot, None),
                None => (Mode::Interp, s.wasm_bytes, Some(DemoteReason::NoAotImage)),
            },
        };
        chosen.push(entry);
    }

    if fast_bytes > free_bytes {
        return Err(PlacementError::FastDoesNotFit {
            needed: fast_bytes,
            free: free_bytes,
        });
    }

    let mut needed: usize = chosen.iter().map(|c| c.1).sum();

    if needed > free_bytes {
        let mut candidates: Vec<usize> = (0..sheets.len())
            .filter(|&i| sheets[i].speed == SheetSpeed::Normal && chosen[i].0 == Mode::Aot)
            .collect();
        candidates.sort_by_key(|&i| {
            let s = &sheets[i];
            (std::cmp::Reverse(s.period_ms), s.step_interp_us, s.number)
        });
        for i in candidates {
            if needed <= free_bytes {
                break;
            }
            needed = needed - chosen[i].1 + sheets[i].wasm_bytes;
            chosen[i] = (
                Mode::Interp,
                sheets[i].wasm_bytes,
                Some(DemoteReason::NoSpace),
            );
        }
        if needed > free_bytes {
            return Err(PlacementError::DoesNotFit {
                needed,
                free: free_bytes,
            });
        }
    }

    let placed = sheets
        .iter()
        .zip(chosen)
        .map(|(s, (mode, bytes, demoted))| {
            let step_us = match mode {
                Mode::Aot => s.step_aot_us,
                Mode::Interp => s.step_interp_us,
            };
            Placed {
                number: s.number,
                mode,
                bytes,
                demoted,
                step_us,
                exceeds_period: u64::from(step_us) > u64::from(s.period_ms) * 1000,
            }
        })
        .collect();

    Ok(Placement {
        sheets: placed,
        needed,
        free: free_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet(number: u32, speed: SheetSpeed) -> SheetImage {
        SheetImage {
            number,
            speed,
            wasm_bytes: 1000,
            aot_bytes: Some(3000),
            period_ms: 10,
            step_interp_us: 100,
            step_aot_us: 10,
        }
    }

    fn get(p: &Placement, number: u32) -> &Placed {
        p.sheets.iter().find(|s| s.number == number).unwrap()
    }

    #[test]
    fn everything_fits() {
        let sheets = [
            sheet(1, SheetSpeed::Fast),
            sheet(2, SheetSpeed::Normal),
            sheet(3, SheetSpeed::Background),
        ];
        let p = place(&sheets, 100_000).unwrap();
        assert_eq!(get(&p, 1).mode, Mode::Aot);
        assert_eq!(get(&p, 2).mode, Mode::Aot);
        assert_eq!(get(&p, 3).mode, Mode::Interp);
        assert_eq!(p.demoted().count(), 0);
        assert_eq!(p.needed, 3000 + 3000 + 1000);
        assert_eq!(p.free, 100_000);
    }

    #[test]
    fn background_is_interp_with_plenty_of_space() {
        let p = place(&[sheet(1, SheetSpeed::Background)], usize::MAX).unwrap();
        assert_eq!(p.sheets[0].mode, Mode::Interp);
        assert_eq!(p.sheets[0].bytes, 1000);
        assert_eq!(p.sheets[0].demoted, None);
    }

    #[test]
    fn longest_period_demoted_first_then_lighter_step() {
        let mut a = sheet(1, SheetSpeed::Normal);
        a.period_ms = 100;
        let mut b = sheet(2, SheetSpeed::Normal);
        b.period_ms = 10;
        // Need 6000, free 4000: one demotion (-2000) is enough -> sheet 1.
        let p = place(&[a, b], 4000).unwrap();
        assert_eq!(get(&p, 1).mode, Mode::Interp);
        assert_eq!(get(&p, 1).demoted, Some(DemoteReason::NoSpace));
        assert_eq!(get(&p, 2).mode, Mode::Aot);

        // Equal periods: the lighter interpreter step goes first.
        let mut c = sheet(1, SheetSpeed::Normal);
        c.step_interp_us = 500;
        let mut d = sheet(2, SheetSpeed::Normal);
        d.step_interp_us = 50;
        let p = place(&[c, d], 4000).unwrap();
        assert_eq!(get(&p, 2).mode, Mode::Interp);
        assert_eq!(get(&p, 1).mode, Mode::Aot);
    }

    #[test]
    fn demotion_stops_as_soon_as_it_fits() {
        let sheets = [
            sheet(1, SheetSpeed::Normal),
            sheet(2, SheetSpeed::Normal),
            sheet(3, SheetSpeed::Normal),
        ];
        // 9000 needed; free 7000 -> one demotion gives 7000.
        let p = place(&sheets, 7000).unwrap();
        assert_eq!(p.demoted().count(), 1);
        assert_eq!(p.needed, 7000);
        assert_eq!(p.sheets.iter().filter(|s| s.mode == Mode::Aot).count(), 2);
    }

    #[test]
    fn fast_is_never_demoted_and_too_big_fails() {
        let sheets = [sheet(1, SheetSpeed::Fast), sheet(2, SheetSpeed::Normal)];
        let p = place(&sheets, 4000).unwrap();
        assert_eq!(get(&p, 1).mode, Mode::Aot);
        assert_eq!(get(&p, 2).mode, Mode::Interp);

        let err = place(&[sheet(1, SheetSpeed::Fast)], 2999).unwrap_err();
        assert_eq!(
            err,
            PlacementError::FastDoesNotFit {
                needed: 3000,
                free: 2999
            }
        );
    }

    #[test]
    fn fast_without_aot_is_error() {
        let mut s = sheet(7, SheetSpeed::Fast);
        s.aot_bytes = None;
        let err = place(&[s], 100_000).unwrap_err();
        assert_eq!(err, PlacementError::FastWithoutAot { number: 7 });
        assert!(err.to_string().contains("лист 7"));
    }

    #[test]
    fn normal_without_aot_is_interp() {
        let mut s = sheet(1, SheetSpeed::Normal);
        s.aot_bytes = None;
        let p = place(&[s], 100_000).unwrap();
        assert_eq!(p.sheets[0].mode, Mode::Interp);
        assert_eq!(p.sheets[0].demoted, Some(DemoteReason::NoAotImage));
        assert_eq!(p.sheets[0].bytes, 1000);
        assert_eq!(p.demoted().count(), 1);
    }

    #[test]
    fn nothing_fits_after_demoting_all() {
        let sheets = [
            sheet(1, SheetSpeed::Normal),
            sheet(2, SheetSpeed::Background),
        ];
        let err = place(&sheets, 1500).unwrap_err();
        assert_eq!(
            err,
            PlacementError::DoesNotFit {
                needed: 2000,
                free: 1500
            }
        );
    }

    #[test]
    fn exceeds_period_uses_chosen_mode() {
        let mut s = sheet(1, SheetSpeed::Normal);
        s.period_ms = 1; // 1000 us
        s.step_interp_us = 1500;
        s.step_aot_us = 200;
        let p = place(&[s], 100_000).unwrap();
        assert_eq!(p.sheets[0].step_us, 200);
        assert!(!p.sheets[0].exceeds_period);

        let p = place(&[s], 1000).unwrap();
        assert_eq!(p.sheets[0].mode, Mode::Interp);
        assert_eq!(p.sheets[0].step_us, 1500);
        assert!(p.sheets[0].exceeds_period);
    }

    #[test]
    fn result_keeps_input_order() {
        let mut a = sheet(5, SheetSpeed::Normal);
        a.period_ms = 100;
        let sheets = [
            a,
            sheet(2, SheetSpeed::Background),
            sheet(9, SheetSpeed::Fast),
        ];
        let p = place(&sheets, 5000).unwrap();
        let order: Vec<u32> = p.sheets.iter().map(|s| s.number).collect();
        assert_eq!(order, vec![5, 2, 9]);
    }
}
