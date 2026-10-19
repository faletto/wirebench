use rand_chacha::ChaCha8Rng;
use rand_core::{RngCore, SeedableRng};

/// Per-device RNG. The seed depends only on the world seed and the device's stable id,
/// never on creation order or on other devices' draws.
#[derive(Debug, Clone)]
pub struct DeviceRng(ChaCha8Rng);

impl DeviceRng {
    pub fn new(world_seed: u64, device: u32) -> Self {
        let mut seed = [0u8; 32];
        seed[..8].copy_from_slice(&world_seed.to_le_bytes());
        seed[8..12].copy_from_slice(&device.to_le_bytes());
        DeviceRng(ChaCha8Rng::from_seed(seed))
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0.next_u64()
    }

    pub fn next_u32(&mut self) -> u32 {
        self.0.next_u32()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_rng_independent_of_creation_order() {
        let mut a1 = DeviceRng::new(7, 1);
        let mut a2 = DeviceRng::new(7, 2);
        let x1 = a1.next_u64();
        let x2 = a2.next_u64();
        // Opposite creation and draw order.
        let mut b2 = DeviceRng::new(7, 2);
        let mut b1 = DeviceRng::new(7, 1);
        assert_eq!(b2.next_u64(), x2);
        assert_eq!(b1.next_u64(), x1);
        assert_ne!(x1, x2);
    }
}
