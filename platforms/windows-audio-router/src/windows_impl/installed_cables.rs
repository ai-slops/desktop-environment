//! Recognition of third-party installed drivers is isolated from routing/DSP.
//! Future own-driver discovery can be added here; manually associated pairs
//! use the same backend without passing through any vendor-specific code.
use audio_cable_core::{Cable, Endpoint, EndpointFlow};

pub(super) fn setup_links() -> Vec<audio_cable_core::SetupLink> {
    vec![audio_cable_core::SetupLink {
        label: "VB-CABLE 공식 설치 안내".into(),
        url: "https://vb-audio.com/Cable/index.htm".into(),
    }]
}

pub(super) fn discover(endpoints: &[Endpoint]) -> Vec<Cable> {
    endpoints
        .iter()
        .filter(|e| e.flow == EndpointFlow::Playback)
        .filter_map(|input| {
            let name = input.name.to_ascii_lowercase();
            if !name.starts_with("cable ") || !name.contains("vb-audio") || !name.contains("input")
            {
                return None;
            }
            let expected = name.replacen("input", "output", 1);
            let mut captures = endpoints.iter().filter(|e| {
                e.flow == EndpointFlow::Capture && e.name.to_ascii_lowercase() == expected
            });
            let capture = captures.next()?;
            if captures.next().is_some() {
                return None;
            }
            Some(Cable {
                provider: "vb-audio.installed".into(),
                name: input.name.clone(),
                playback_id: input.id.clone(),
                capture_id: capture.id.clone(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovery_requires_both_directions_and_does_not_guess_ambiguous_pairs() {
        let playback = Endpoint {
            id: "render".into(),
            name: "CABLE Input (VB-Audio Virtual Cable)".into(),
            flow: EndpointFlow::Playback,
            is_default: false,
        };
        let capture = Endpoint {
            id: "capture".into(),
            name: "CABLE Output (VB-Audio Virtual Cable)".into(),
            flow: EndpointFlow::Capture,
            is_default: false,
        };
        assert!(discover(&[playback.clone()]).is_empty());
        assert_eq!(discover(&[playback.clone(), capture.clone()]).len(), 1);
        assert!(discover(&[playback, capture.clone(), capture]).is_empty());
    }
}
