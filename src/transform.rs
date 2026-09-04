//! Transform Withings measure groups into Garmin-bound readings (ticket 07).
//!
//! Weight (meastype 1) decodes to kilograms. Blood pressure groups the
//! systolic (10), diastolic (9), and pulse (11) measures of one measure group
//! into a single reading; a reading is emitted only when systolic and
//! diastolic are both present, and any component outside Garmin's validation
//! ranges skips the whole reading with a warning.

use crate::withings::{decode_value, MeasureGroup, RawMeasure};

/// Garmin's validation ranges (inclusive).
pub const SYSTOLIC_MIN: f64 = 70.0;
pub const SYSTOLIC_MAX: f64 = 260.0;
pub const DIASTOLIC_MIN: f64 = 40.0;
pub const DIASTOLIC_MAX: f64 = 150.0;
pub const PULSE_MIN: f64 = 20.0;
pub const PULSE_MAX: f64 = 250.0;

#[derive(Debug, Clone, PartialEq)]
pub struct WeightReading {
    pub kg: f64,
    pub epoch: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BpReading {
    pub systolic: f64,
    pub diastolic: f64,
    pub pulse: Option<f64>,
    pub epoch: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Metric {
    Weight,
    BloodPressure,
}

/// A reading that was dropped, with a human-readable reason for the report.
#[derive(Debug, Clone, PartialEq)]
pub struct Skip {
    pub metric: Metric,
    pub epoch: i64,
    pub reason: String,
}

/// Split measure groups into weight readings, blood-pressure readings, and
/// skipped readings. Both lists come back sorted by time ascending.
pub fn transform(groups: Vec<MeasureGroup>) -> (Vec<WeightReading>, Vec<BpReading>, Vec<Skip>) {
    let mut weights = Vec::new();
    let mut bps = Vec::new();
    let mut skips = Vec::new();

    for group in groups {
        for measure in &group.measures {
            if measure.meastype == 1 {
                weights.push(WeightReading {
                    kg: round3(decode_value(measure.value, measure.unit)),
                    epoch: group.date,
                });
            }
        }
        match pair_blood_pressure(&group.measures) {
            None => {} // no BP measures in this group at all
            Some(Ok(mut reading)) => {
                reading.epoch = group.date;
                bps.push(reading);
            }
            Some(Err(reason)) => skips.push(Skip {
                metric: Metric::BloodPressure,
                epoch: group.date,
                reason,
            }),
        }
    }

    weights.sort_by_key(|w| w.epoch);
    bps.sort_by_key(|b| b.epoch);
    (weights, bps, skips)
}

/// Pair one group's BP measures into a reading. `None` means the group holds
/// no BP measures at all; `Some(Err(..))` is a reading that must be skipped
/// with that reason.
fn pair_blood_pressure(measures: &[RawMeasure]) -> Option<Result<BpReading, String>> {
    let mut systolic: Option<f64> = None;
    let mut diastolic: Option<f64> = None;
    let mut pulse: Option<f64> = None;
    for measure in measures {
        match measure.meastype {
            10 => systolic = Some(decode_value(measure.value, measure.unit)),
            9 => diastolic = Some(decode_value(measure.value, measure.unit)),
            11 => pulse = Some(decode_value(measure.value, measure.unit)),
            _ => {}
        }
    }

    if systolic.is_none() && diastolic.is_none() && pulse.is_none() {
        return None;
    }

    let (Some(systolic), Some(diastolic)) = (systolic, diastolic) else {
        return Some(Err(
            "blood-pressure group lacks paired systolic and diastolic".to_string(),
        ));
    };

    if !(SYSTOLIC_MIN..=SYSTOLIC_MAX).contains(&systolic) {
        return Some(Err(format!(
            "systolic {systolic} out of range ({SYSTOLIC_MIN}..{SYSTOLIC_MAX})"
        )));
    }
    if !(DIASTOLIC_MIN..=DIASTOLIC_MAX).contains(&diastolic) {
        return Some(Err(format!(
            "diastolic {diastolic} out of range ({DIASTOLIC_MIN}..{DIASTOLIC_MAX})"
        )));
    }
    if let Some(pulse) = pulse {
        if !(PULSE_MIN..=PULSE_MAX).contains(&pulse) {
            return Some(Err(format!(
                "pulse {pulse} out of range ({PULSE_MIN}..{PULSE_MAX})"
            )));
        }
    }

    Some(Ok(BpReading {
        systolic,
        diastolic,
        pulse,
        epoch: 0,
    }))
}

/// Round to three decimals, guarding against float representation artifacts
/// (`82.456` stays `82.456`). Live verification (ticket 11) confirmed Garmin
/// accepts multi-decimal kg values, so Withings' fidelity is kept.
fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measure(meastype: u32, value: i64, unit: i32) -> RawMeasure {
        RawMeasure {
            meastype,
            value,
            unit,
        }
    }

    #[test]
    fn weight_decodes_to_kg_and_rounds() {
        let (weights, bps, skips) = transform(vec![MeasureGroup {
            date: 100,
            measures: vec![measure(1, 82456, -3)],
        }]);
        assert_eq!(weights.len(), 1);
        assert_eq!(weights[0].kg, 82.456);
        assert!(bps.is_empty());
        assert!(skips.is_empty());
    }

    #[test]
    fn bp_groups_systolic_diastolic_and_pulse() {
        let (_w, bps, skips) = transform(vec![MeasureGroup {
            date: 200,
            measures: vec![measure(10, 120, 0), measure(9, 80, 0), measure(11, 72, 0)],
        }]);
        assert_eq!(bps.len(), 1);
        assert_eq!(bps[0].systolic, 120.0);
        assert_eq!(bps[0].diastolic, 80.0);
        assert_eq!(bps[0].pulse, Some(72.0));
        assert!(skips.is_empty());
    }

    #[test]
    fn bp_group_without_pair_skipped_but_weight_group_untouched() {
        let (weights, bps, skips) = transform(vec![
            MeasureGroup {
                date: 200,
                measures: vec![measure(10, 120, 0)],
            },
            MeasureGroup {
                date: 300,
                measures: vec![measure(1, 80000, -3)],
            },
        ]);
        assert!(bps.is_empty());
        assert_eq!(weights.len(), 1);
        assert_eq!(skips.len(), 1);
        assert_eq!(skips[0].metric, Metric::BloodPressure);
        assert!(skips[0].reason.contains("paired"), "{}", skips[0].reason);
    }

    #[test]
    fn out_of_range_components_skip_whole_reading() {
        let (_w, bps, skips) = transform(vec![
            MeasureGroup {
                date: 200,
                measures: vec![measure(10, 300, 0), measure(9, 80, 0)],
            },
            MeasureGroup {
                date: 210,
                measures: vec![measure(10, 120, 0), measure(9, 20, 0)],
            },
            MeasureGroup {
                date: 220,
                measures: vec![measure(10, 120, 0), measure(9, 80, 0), measure(11, 500, 0)],
            },
        ]);
        assert!(bps.is_empty());
        assert_eq!(skips.len(), 3);
        assert!(skips[0].reason.contains("systolic"), "{}", skips[0].reason);
        assert!(skips[1].reason.contains("diastolic"), "{}", skips[1].reason);
        assert!(skips[2].reason.contains("pulse"), "{}", skips[2].reason);
    }

    #[test]
    fn boundary_values_pass() {
        let (_w, bps, _skips) = transform(vec![MeasureGroup {
            date: 200,
            measures: vec![measure(10, 260, 0), measure(9, 40, 0), measure(11, 250, 0)],
        }]);
        assert_eq!(bps.len(), 1);
    }

    #[test]
    fn readings_come_back_time_ordered() {
        let (weights, _bps, _skips) = transform(vec![
            MeasureGroup {
                date: 500,
                measures: vec![measure(1, 80000, -3)],
            },
            MeasureGroup {
                date: 100,
                measures: vec![measure(1, 81000, -3)],
            },
        ]);
        assert_eq!(weights[0].epoch, 100);
        assert_eq!(weights[1].epoch, 500);
    }
}
