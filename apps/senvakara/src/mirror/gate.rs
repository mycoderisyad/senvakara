//! Which packets of the stream are decoded, so a hidden pane does no decoding at all.
//!
//! H.264 frames depend on the ones before them, so after packets were skipped the decoder can
//! only restart from a key frame that carries its own parameter sets (SPS/PPS). The adapter
//! prepends those only to the first packet after a config packet, which is what the phone sends
//! after a "reset video" request, so that is the one packet that is safe to restart from.

/// What to do with one packet.
#[derive(Debug, PartialEq, Eq)]
pub enum Admit {
    /// Drop it.
    Skip,
    /// Decode it with the decoder as it is.
    Decode,
    /// Decode it with a new decoder: everything before it was skipped.
    DecodeFresh,
}

#[derive(Default)]
pub struct Gate {
    /// Packets were skipped, so decoding resumes only at a self-contained key frame.
    waiting: bool,
}

impl Gate {
    /// `key`: the packet is a key frame. `config`: it carries new parameter sets.
    pub fn admit(&mut self, visible: bool, key: bool, config: bool) -> Admit {
        if !visible {
            self.waiting = true;
            return Admit::Skip;
        }
        if !self.waiting {
            return Admit::Decode;
        }
        if key && config {
            self.waiting = false;
            Admit::DecodeFresh
        } else {
            Admit::Skip
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_visible_stream_is_decoded_as_it_comes() {
        let mut gate = Gate::default();
        assert_eq!(gate.admit(true, true, true), Admit::Decode);
        assert_eq!(gate.admit(true, false, false), Admit::Decode);
    }

    #[test]
    fn a_hidden_stream_is_dropped_whatever_it_holds() {
        let mut gate = Gate::default();
        assert_eq!(gate.admit(false, true, true), Admit::Skip);
        assert_eq!(gate.admit(false, false, false), Admit::Skip);
    }

    #[test]
    fn decoding_resumes_only_at_a_key_frame_with_parameter_sets() {
        let mut gate = Gate::default();
        assert_eq!(gate.admit(false, false, false), Admit::Skip);
        // Shown again: delta frames, and a key frame without parameter sets, stay skipped.
        assert_eq!(gate.admit(true, false, false), Admit::Skip);
        assert_eq!(gate.admit(true, true, false), Admit::Skip);
        assert_eq!(gate.admit(true, true, true), Admit::DecodeFresh);
        assert_eq!(gate.admit(true, false, false), Admit::Decode);
    }

    #[test]
    fn hiding_again_before_the_key_frame_keeps_waiting() {
        let mut gate = Gate::default();
        gate.admit(false, false, false);
        assert_eq!(gate.admit(true, false, false), Admit::Skip);
        assert_eq!(gate.admit(false, true, true), Admit::Skip);
        assert_eq!(gate.admit(true, true, true), Admit::DecodeFresh);
    }

    #[test]
    fn a_pane_shown_without_losing_packets_keeps_its_decoder() {
        let mut gate = Gate::default();
        assert_eq!(gate.admit(true, false, false), Admit::Decode);
    }
}
