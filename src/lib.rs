pub mod combat;
pub mod core;
pub mod engine;
pub mod farm;
pub mod movement;
pub mod ui;
pub mod visuals;

use crate::combat::{AimAssist, RecoilControl, SilentAim, TriggerBot};
use crate::core::arbiter::ActionArbiter;
use crate::core::events::EventBus;
use crate::core::registry::ModuleRegistry;
use crate::core::snapshot::GameSnapshot;
use crate::engine::action_executor::ActionExecutor;
use crate::engine::input::{InputEvent, InputHandler};
use crate::engine::jni_bridge::{self, JniBridge};
use crate::engine::render_hook;
use crate::engine::renderer::Renderer;
use crate::farm::{AutoArmor, AutoFarm, AutoLoot};
use crate::movement::{Fly, NoFall, SafeWalk, Speed, WaterWalk};
use crate::ui::clickgui::ClickGUI;
use crate::ui::hud::HUD;
use crate::visuals::{PlayerESP, Radar, SkeletonESP, WorldESP};

pub struct VibeClient {
    pub registry: ModuleRegistry,
    pub arbiter: ActionArbiter,
    pub event_bus: EventBus,
    pub click_gui: ClickGUI,
    pub renderer: Renderer,
    pub jni: JniBridge,
    pub snapshot: GameSnapshot,
    pub input: InputHandler,
    pub is_running: bool,
}

impl Default for VibeClient {
    fn default() -> Self { Self::new() }
}

impl VibeClient {
    pub fn new() -> Self {
        let mut registry = ModuleRegistry::new();

        // Combat
        registry.register(AimAssist::new());
        registry.register(RecoilControl::new());
        registry.register(TriggerBot::new());
        registry.register(SilentAim::new());

        // Visuals
        registry.register(PlayerESP::new());
        registry.register(SkeletonESP::new());
        registry.register(WorldESP::new());
        registry.register(Radar::new());
        registry.register(HUD::new());

        // Farm
        registry.register(AutoLoot::new());
        registry.register(AutoArmor::new());
        registry.register(AutoFarm::new());

        // Movement
        registry.register(Speed::new());
        registry.register(Fly::new());
        registry.register(NoFall::new());
        registry.register(SafeWalk::new());
        registry.register(WaterWalk::new());

        Self {
            registry,
            arbiter: ActionArbiter::new(),
            event_bus: EventBus::new(),
            click_gui: ClickGUI::new(),
            renderer: Renderer::new(1920.0, 1080.0),
            jni: JniBridge::new(),
            snapshot: GameSnapshot::default(),
            input: InputHandler::new(),
            is_running: true,
        }
    }

    pub fn on_tick(&mut self, tick: u64) {
        if !self.is_running { return; }
        if !self.snapshot.local_player.entity.is_alive { return; }
        self.snapshot.tick_count = tick;

        // TriggerBot
        if let Some(tb) = self.registry.get_mut("TriggerBot") {
            if let Some(trigger_bot) = tb.as_any_mut().downcast_mut::<TriggerBot>() {
                trigger_bot.check_trigger(&self.snapshot, &mut self.arbiter, tick);
            }
        }

        // AutoLoot
        if let Some(al) = self.registry.get_mut("AutoLoot") {
            if let Some(auto_loot) = al.as_any_mut().downcast_mut::<AutoLoot>() {
                auto_loot.process_container_loot(&self.snapshot, &mut self.arbiter, tick);
            }
        }

        // AutoArmor
        if let Some(aa) = self.registry.get_mut("AutoArmor") {
            if let Some(auto_armor) = aa.as_any_mut().downcast_mut::<AutoArmor>() {
                auto_armor.check_and_equip(&self.snapshot, &mut self.arbiter, tick);
            }
        }

        // AutoFarm
        if let Some(af) = self.registry.get_mut("AutoFarm") {
            if let Some(auto_farm) = af.as_any_mut().downcast_mut::<AutoFarm>() {
                auto_farm.process_farming(&self.snapshot, &mut self.arbiter, tick);
            }
        }

        // NoFall
        if let Some(nf) = self.registry.get_mut("NoFall") {
            if let Some(no_fall) = nf.as_any_mut().downcast_mut::<NoFall>() {
                no_fall.check_and_apply(&self.snapshot, &mut self.arbiter);
            }
        }
    }

    pub fn on_render(&mut self) {
        let vw = render_hook::VIEWPORT_WIDTH.load(std::sync::atomic::Ordering::Relaxed) as f32;
        let vh = render_hook::VIEWPORT_HEIGHT.load(std::sync::atomic::Ordering::Relaxed) as f32;
        self.renderer.begin_frame(vw.max(640.0), vh.max(480.0));

        // Render ESP
        if let Some(m) = self.registry.get_mut("PlayerESP") {
            if let Some(esp) = m.as_any_mut().downcast_mut::<PlayerESP>() {
                esp.update_render_list(&self.snapshot);
                self.renderer.render_player_esp(esp);
            }
        }
        if let Some(m) = self.registry.get_mut("SkeletonESP") {
            if let Some(skel) = m.as_any_mut().downcast_mut::<SkeletonESP>() {
                skel.update_skeletons(&self.snapshot);
                self.renderer.render_skeletons(skel);
            }
        }
        if let Some(m) = self.registry.get_mut("WorldESP") {
            if let Some(world) = m.as_any_mut().downcast_mut::<WorldESP>() {
                world.update_world_items(&self.snapshot);
                self.renderer.render_world_esp(world);
            }
        }
        if let Some(m) = self.registry.get_mut("Radar") {
            if let Some(radar) = m.as_any_mut().downcast_mut::<Radar>() {
                radar.update_blips(&self.snapshot);
                self.renderer.render_radar(radar);
            }
        }
        if let Some(m) = self.registry.get_mut("HUD") {
            if let Some(hud) = m.as_any_mut().downcast_mut::<HUD>() {
                self.renderer.render_hud(hud);
            }
        }

        // Render ClickGUI
        self.renderer.render_clickgui(&self.click_gui, &self.registry);
    }
}

// ══════════════════════════════════════════════════════════════════════
// Client Main Loop (runs on injected thread)
// ══════════════════════════════════════════════════════════════════════

unsafe fn client_main() {
    // Wait for Astraea to finish its init hooks
    std::thread::sleep(std::time::Duration::from_millis(2500));

    jni_bridge::log_msg("=== Vibe Client v0.1.0 starting ===");

    // Install VEH handler for crash-proof JNI
    jni_bridge::install_veh();

    // Create client
    let mut client = VibeClient::new();

    // Attach to JVM
    match client.jni.attach_to_game_process() {
        Ok(()) => {
            jni_bridge::log_msg("[CLIENT] JVM attached successfully");
        }
        Err(e) => {
            jni_bridge::log_msg(&format!("[CLIENT] JVM attach failed: {} — retrying in 3s", e));
            std::thread::sleep(std::time::Duration::from_millis(3000));
            if let Err(e2) = client.jni.attach_to_game_process() {
                jni_bridge::log_msg(&format!("[CLIENT] JVM attach failed again: {} — aborting", e2));
                return;
            }
        }
    }

    // Install render hook
    if render_hook::install_hook() {
        jni_bridge::log_msg("[CLIENT] Render hook installed");
    } else {
        jni_bridge::log_msg("[CLIENT] Render hook failed — overlay won't draw, but logic continues");
    }

    // Enable some default modules
    if let Some(m) = client.registry.get_mut("HUD") { m.set_enabled(true); }
    if let Some(m) = client.registry.get_mut("PlayerESP") { m.set_enabled(true); }

    jni_bridge::log_msg("[CLIENT] Entering main loop (20 TPS)");

    let mut tick: u64 = 0;
    let tick_interval = std::time::Duration::from_millis(50); // 20 TPS
    let mut current_mouse_pos = crate::core::snapshot::Vector2::ZERO;

    while client.is_running {
        let tick_start = std::time::Instant::now();

        // 1. Read game state from JVM
        client.jni.read_game_state(&mut client.snapshot);

        // 2. Poll input
        let events = client.input.poll();
        for event in events {
            match event {
                InputEvent::ToggleGUI => {
                    let open = client.click_gui.toggle();
                    unsafe { jni_bridge::set_cursor_unlocked(open); }
                    client.jni.set_mouse_grabbed(!open);
                    jni_bridge::log_msg(&format!("[GUI] ClickGUI toggled: {}", if open { "OPEN" } else { "CLOSED" }));
                }
                InputEvent::ToggleModule(name) => {
                    if let Some(m) = client.registry.get_mut(&name) {
                        let new_state = !m.is_enabled();
                        m.set_enabled(new_state);
                        jni_bridge::log_msg(&format!("[INPUT] {} -> {}", name, if new_state { "ON" } else { "OFF" }));
                    }
                }
                InputEvent::Panic => {
                    jni_bridge::log_msg("[INPUT] PANIC — disabling all modules");
                    for m in client.registry.all_mut() {
                        m.set_enabled(false);
                    }
                    client.click_gui.is_open = false;
                    unsafe { jni_bridge::set_cursor_unlocked(false); }
                    client.jni.set_mouse_grabbed(true);
                }
                InputEvent::MousePosition(x, y) => {
                    current_mouse_pos = crate::core::snapshot::Vector2::new(x, y);
                    client.click_gui.handle_mouse_move(current_mouse_pos);
                }
                InputEvent::MouseClick { button, pressed } => {
                    if client.click_gui.is_open {
                        client.click_gui.handle_mouse_click(
                            current_mouse_pos,
                            button,
                            pressed,
                            &mut client.registry,
                        );
                    }
                }
                InputEvent::KeyPress(key_code) => {
                    if client.click_gui.is_open {
                        client.click_gui.handle_key_press(key_code, &mut client.registry);
                    }
                }
            }
        }

        // 3. Run module tick logic
        client.on_tick(tick);

        // 4. Execute pending actions from arbiter
        loop {
            match client.arbiter.on_tick(tick) {
                Some(action) => ActionExecutor::execute(&action, &client.jni),
                None => break,
            }
        }

        // 5. Render (build draw commands + submit to hook)
        client.on_render();
        render_hook::submit_frame(
            &client.renderer.command_buffer,
            &client.renderer.string_pool,
        );

        if tick % 100 == 0 {
            let p = &client.snapshot.local_player.entity.position;
            jni_bridge::log_msg(&format!(
                "[CLIENT] Tick {} | Player=({:.1}, {:.1}, {:.1}) HP={:.1} | Entities={}",
                tick, p.x, p.y, p.z, client.snapshot.local_player.entity.health, client.snapshot.entities.len()
            ));
        }

        tick += 1;

        // Throttle to target TPS
        let elapsed = tick_start.elapsed();
        if elapsed < tick_interval {
            std::thread::sleep(tick_interval - elapsed);
        }
    }

    jni_bridge::log_msg("=== Vibe Client shutting down ===");
}

// ══════════════════════════════════════════════════════════════════════
// DLL Entry Point
// ══════════════════════════════════════════════════════════════════════

#[no_mangle]
#[cfg(target_os = "windows")]
pub unsafe extern "system" fn DllMain(
    _hinst_dll: *mut std::ffi::c_void,
    fdw_reason: u32,
    _lpv_reserved: *mut std::ffi::c_void,
) -> i32 {
    const DLL_PROCESS_ATTACH: u32 = 1;

    if fdw_reason == DLL_PROCESS_ATTACH {
        std::thread::spawn(|| {
            client_main();
        });
    }

    1
}
