use crate::core::snapshot::Rotation;

pub struct AntiCheatEvasion;

impl AntiCheatEvasion {
    pub fn xor_cipher(data: &[u8], key: u8) -> Vec<u8> {
        data.iter().map(|b| b ^ key).collect()
    }

    #[cfg(target_os = "windows")]
    pub unsafe fn hide_pe_header(module_base: *mut u8) -> bool {
        #[link(name = "kernel32")]
        extern "system" {
            fn VirtualProtect(
                lpAddress: *mut std::ffi::c_void,
                dwSize: usize,
                flNewProtect: u32,
                lpflOldProtect: *mut u32,
            ) -> i32;
        }

        if module_base.is_null() {
            return false;
        }

        const PAGE_EXECUTE_READWRITE: u32 = 0x40;
        let mut old_protect = 0u32;

        if VirtualProtect(module_base as *mut std::ffi::c_void, 4096, PAGE_EXECUTE_READWRITE, &mut old_protect) != 0 {
            std::ptr::write_bytes(module_base, 0, 4096);
            VirtualProtect(module_base as *mut std::ffi::c_void, 4096, old_protect, &mut old_protect);
            true
        } else {
            false
        }
    }

    #[cfg(not(target_os = "windows"))]
    pub unsafe fn hide_pe_header(_module_base: *mut u8) -> bool {
        false
    }

    pub fn generate_bezier_mouse_trajectory(
        start: Rotation,
        target: Rotation,
        steps: usize,
        randomness: f32,
    ) -> Vec<Rotation> {
        let n_steps = steps.max(3);
        let mut trajectory = Vec::with_capacity(n_steps);

        let delta = start.delta_to(&target);

        // Control point 1: 30% along path with random deviation
        let cp1_pitch = start.pitch + delta.pitch * 0.3 + (randomness * 1.5 - 0.75);
        let cp1_yaw = start.yaw + delta.yaw * 0.3 + (randomness * 2.0 - 1.0);

        // Control point 2: 70% along path with slight deviation
        let cp2_pitch = start.pitch + delta.pitch * 0.7 + (randomness * 0.8 - 0.4);
        let cp2_yaw = start.yaw + delta.yaw * 0.7 + (randomness * 1.0 - 0.5);

        let p0 = (start.pitch, start.yaw);
        let p1 = (cp1_pitch, cp1_yaw);
        let p2 = (cp2_pitch, cp2_yaw);
        let p3 = (target.pitch, target.yaw);

        for i in 1..=n_steps {
            let t = i as f32 / n_steps as f32;
            let inv_t = 1.0 - t;

            let b0 = inv_t.powi(3);
            let b1 = 3.0 * inv_t.powi(2) * t;
            let b2 = 3.0 * inv_t * t.powi(2);
            let b3 = t.powi(3);

            let cur_pitch = b0 * p0.0 + b1 * p1.0 + b2 * p2.0 + b3 * p3.0;
            let cur_yaw = b0 * p0.1 + b1 * p1.1 + b2 * p2.1 + b3 * p3.1;

            trajectory.push(Rotation::new(cur_pitch, cur_yaw));
        }

        trajectory
    }

    pub fn calculate_packet_delay_jitter(base_ms: u64, tick: u64) -> u64 {
        let jitter = (tick % 7) as i64 - 3; // -3ms to +3ms pseudo-random jitter
        ((base_ms as i64 + jitter).max(1)) as u64
    }
}
