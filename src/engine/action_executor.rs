//! Executes QueuedActions from the ActionArbiter via JNI.

use crate::core::arbiter::{ActionPayload, QueuedAction};
use crate::engine::jni_bridge::JniBridge;

pub struct ActionExecutor;

impl ActionExecutor {
    /// Execute a single queued action through the JNI bridge.
    pub fn execute(action: &QueuedAction, bridge: &JniBridge) {
        if !bridge.is_connected { return; }

        match &action.payload {
            ActionPayload::AimAdjustment { pitch, yaw, smooth_steps: _ } => {
                // For smooth aim, we apply the final rotation.
                // Bezier smoothing is done at the module level (evasion::generate_bezier_mouse_trajectory),
                // so by the time an AimAdjustment action reaches here, it's the target for this tick.
                bridge.set_player_rotation(*yaw, *pitch);
            }

            ActionPayload::AttackEntity { target_id, is_headshot: _ } => {
                // Attack is handled by calling PlayerControllerMP.attackEntity
                // For now we trigger it via setting the target and letting MC handle swing.
                // Full implementation requires resolving bsa class and attackEntity method.
                // This will be wired when the dumper provides offsets.
                let _ = target_id;
            }

            ActionPayload::GroundSpoof { on_ground } => {
                bridge.set_on_ground(*on_ground);
            }

            ActionPayload::AdjustVelocity { forward, strafe, vertical } => {
                // Convert forward/strafe into world-space motion
                // This is a simplified version; full version uses player's yaw
                bridge.set_player_motion(*forward as f64, *vertical as f64, *strafe as f64);
            }

            ActionPayload::MoveSlot { from_container: _, slot_from: _, slot_to: _, item_id: _ } => {
                // Container slot manipulation requires resolving Container/Slot classes
                // Deferred until dumper provides exact slot click method signatures
            }

            ActionPayload::EquipArmor { slot_from: _, armor_slot_idx: _ } => {
                // Same as MoveSlot — requires Container class resolution
            }

            ActionPayload::MineBlock { position: _, tool_slot: _ } => {
                // Block breaking requires sending CPacketPlayerDigging
                // Deferred until network packet sending is wired
            }
        }
    }
}
