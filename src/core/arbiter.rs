use std::collections::VecDeque;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ActionPriority {
    Idle = 0,
    Farm = 1,
    Inventory = 2,
    Movement = 3,
    Combat = 4,
    Emergency = 5,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ActionPayload {
    AimAdjustment { pitch: f32, yaw: f32, smooth_steps: u32 },
    AttackEntity { target_id: u32, is_headshot: bool },
    MoveSlot { from_container: bool, slot_from: u16, slot_to: u16, item_id: u32 },
    EquipArmor { slot_from: u16, armor_slot_idx: u8 },
    MineBlock { position: [i32; 3], tool_slot: u8 },
    AdjustVelocity { forward: f32, strafe: f32, vertical: f32 },
    GroundSpoof { on_ground: bool },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedAction {
    pub id: u64,
    pub priority: ActionPriority,
    pub payload: ActionPayload,
    pub scheduled_tick: u64,
    pub timeout_ticks: u32,
}

#[derive(Debug, Default)]
pub struct ActionArbiter {
    action_counter: u64,
    queue: VecDeque<QueuedAction>,
    current_tick: u64,
    last_inventory_action_tick: u64,
    last_attack_action_tick: u64,
    inventory_action_cooldown_ticks: u64,
    attack_cooldown_ticks: u64,
}

impl ActionArbiter {
    pub fn new() -> Self {
        Self {
            action_counter: 0,
            queue: VecDeque::new(),
            current_tick: 0,
            last_inventory_action_tick: 0,
            last_attack_action_tick: 0,
            inventory_action_cooldown_ticks: 2,
            attack_cooldown_ticks: 1,
        }
    }

    pub fn set_inventory_cooldown(&mut self, ticks: u64) {
        self.inventory_action_cooldown_ticks = ticks;
    }

    pub fn set_attack_cooldown(&mut self, ticks: u64) {
        self.attack_cooldown_ticks = ticks;
    }

    pub fn submit_action(&mut self, priority: ActionPriority, payload: ActionPayload) -> u64 {
        self.action_counter += 1;
        let action = QueuedAction {
            id: self.action_counter,
            priority,
            payload,
            scheduled_tick: self.current_tick,
            timeout_ticks: 20,
        };

        let pos = self
            .queue
            .iter()
            .position(|existing| existing.priority < action.priority)
            .unwrap_or(self.queue.len());

        self.queue.insert(pos, action);
        self.action_counter
    }

    pub fn on_tick(&mut self, tick: u64) -> Option<QueuedAction> {
        self.current_tick = tick;

        self.queue.retain(|action| {
            tick <= action.scheduled_tick + (action.timeout_ticks as u64)
        });

        for i in 0..self.queue.len() {
            let action = &self.queue[i];

            let can_execute = match &action.payload {
                ActionPayload::MoveSlot { .. } | ActionPayload::EquipArmor { .. } => {
                    self.last_inventory_action_tick == 0
                        || tick >= self.last_inventory_action_tick + self.inventory_action_cooldown_ticks
                }
                ActionPayload::AttackEntity { .. } => {
                    self.last_attack_action_tick == 0
                        || tick >= self.last_attack_action_tick + self.attack_cooldown_ticks
                }
                _ => true,
            };

            if can_execute {
                let action = self.queue.remove(i).unwrap();
                match &action.payload {
                    ActionPayload::MoveSlot { .. } | ActionPayload::EquipArmor { .. } => {
                        self.last_inventory_action_tick = tick;
                    }
                    ActionPayload::AttackEntity { .. } => {
                        self.last_attack_action_tick = tick;
                    }
                    _ => {}
                }
                return Some(action);
            }
        }

        None
    }

    pub fn clear(&mut self) {
        self.queue.clear();
    }

    pub fn pending_count(&self) -> usize {
        self.queue.len()
    }
}
