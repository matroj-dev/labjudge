#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    NotStarted,
    Running,
    Ended,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotStarted => "not_started",
            Self::Running => "running",
            Self::Ended => "ended",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmissionStatus {
    Queued,
    Compiling,
    Running,
    Done,
    Failed,
}

impl SubmissionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Compiling => "compiling",
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "queued" => Some(Self::Queued),
            "compiling" => Some(Self::Compiling),
            "running" => Some(Self::Running),
            "done" => Some(Self::Done),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Verdict {
    Ac,
    Wa,
    Tle,
    Mle,
    Re,
    Ole,
    Ce,
    Ie,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ac => "AC",
            Self::Wa => "WA",
            Self::Tle => "TLE",
            Self::Mle => "MLE",
            Self::Re => "RE",
            Self::Ole => "OLE",
            Self::Ce => "CE",
            Self::Ie => "IE",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "AC" => Some(Self::Ac),
            "WA" => Some(Self::Wa),
            "TLE" => Some(Self::Tle),
            "MLE" => Some(Self::Mle),
            "RE" => Some(Self::Re),
            "OLE" => Some(Self::Ole),
            "CE" => Some(Self::Ce),
            "IE" => Some(Self::Ie),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_and_verdict_round_trip() {
        for status in [
            SubmissionStatus::Queued,
            SubmissionStatus::Compiling,
            SubmissionStatus::Running,
            SubmissionStatus::Done,
            SubmissionStatus::Failed,
        ] {
            assert_eq!(SubmissionStatus::parse(status.as_str()), Some(status));
        }
        for verdict in [
            Verdict::Ac,
            Verdict::Wa,
            Verdict::Tle,
            Verdict::Mle,
            Verdict::Re,
            Verdict::Ole,
            Verdict::Ce,
            Verdict::Ie,
        ] {
            assert_eq!(Verdict::parse(verdict.as_str()), Some(verdict));
        }
        assert_eq!(SubmissionStatus::parse("nope"), None);
        assert_eq!(Verdict::parse("nope"), None);
    }
}
