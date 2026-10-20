/// Canonical identity and Withings measurement mapping for a sync metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Metric {
    Weight,
    BloodPressure,
}

impl Metric {
    pub const ALL: [Self; 2] = [Self::Weight, Self::BloodPressure];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Weight => "weight",
            Self::BloodPressure => "blood-pressure",
        }
    }

    pub const fn config_key(self) -> &'static str {
        match self {
            Self::Weight => "sync.weight.since",
            Self::BloodPressure => "sync.bp.since",
        }
    }

    pub const fn meastypes(self) -> &'static str {
        match self {
            Self::Weight => "1",
            Self::BloodPressure => "9,10,11",
        }
    }
}
