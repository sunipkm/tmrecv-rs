const PICC_TLM_PRESYNC: [u8; 4] = 0xBAADDAAD_u32.to_le_bytes();
const PICC_TLM_POSTSYNC: [u8; 4] = 0xDEADBEEF_u32.to_le_bytes();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PiccFrameState {
    Empty,
    Presync,
}

#[derive(Debug)]
pub struct PiccFrame {
    frame: Vec<u8>,
    state: PiccFrameState,
}

impl Default for PiccFrame {
    fn default() -> Self {
        PiccFrame {
            frame: Vec::with_capacity(65536),
            state: PiccFrameState::Empty,
        }
    }
}

pub type Packet = Vec<u8>;

impl PiccFrame {
    pub fn push(&mut self, data: &[u8]) -> Result<Vec<Packet>, Packet> {
        self.frame.extend_from_slice(data);
        let mut output = Vec::new();
        loop {
            match self.state {
                PiccFrameState::Empty => {
                    if let Some(pos) = self
                        .frame
                        .windows(PICC_TLM_PRESYNC.len())
                        .position(|window| window == PICC_TLM_PRESYNC)
                    {
                        self.state = PiccFrameState::Presync;
                        self.frame.drain(..pos);
                    } else {
                        break;
                    }
                }
                PiccFrameState::Presync => {
                    let res = {
                        let mut out = None;
                        for (pos, window) in
                            self.frame[PICC_TLM_PRESYNC.len()..] // Safety: We know the frame has at least 4 bytes of pre-sync symbol
                                .windows(PICC_TLM_POSTSYNC.len())
                                .enumerate()
                        {
                            if window == PICC_TLM_POSTSYNC {
                                out = Some(Ok(pos
                                    + PICC_TLM_PRESYNC.len()
                                    + PICC_TLM_POSTSYNC.len()));
                                self.state = PiccFrameState::Empty;
                            } else if window == PICC_TLM_PRESYNC {
                                out = Some(Err(pos + PICC_TLM_PRESYNC.len()));
                            }
                        }
                        out
                    };
                    if let Some(res) = res {
                        match res {
                            Ok(end) => {
                                let packet = self.frame.drain(..end).collect();
                                output.push(packet);
                            }
                            Err(end) => {
                                let malformed = self.frame.drain(..end).collect();
                                return Err(malformed);
                            }
                        }
                    } else {
                        // nothing found, get out
                        break;
                    }
                }
            }
        }
        Ok(output)
    }
}
