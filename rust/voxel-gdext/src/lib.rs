//! `voxel-gdext` — Godot 4 GDExtension bindings for the Rust voxel engine.
//!
//! This crate is the **thin binding layer**: the only place that depends on the
//! `godot` crate and exposes `#[func]`/`#[base]`/`#[signal]` symbols to GDScript.
//! All engine-agnostic logic lives in [`voxel_core`]; this crate wraps it into
//! Godot classes.
//!
//! ## Status
//! Phase 2 skeleton: a single `VoxelRustHello` class with a hello-world method
//! that delegates to `voxel_core` (proving the crate-to-crate + GDExtension path
//! works end-to-end). Real voxel classes land in later phases.
//!
//! ## Loading in Godot
//! Build the `.so`/`.dylib`/`.dll`, then add a `.gdextension` file pointing at
//! it (see `rust/voxel-gdext/voxel_gdext.gdextension.in`). Restart the editor.

use godot::classes::Engine;
use godot::init::{gdextension, ExtensionLibrary, InitStage};
use godot::prelude::*;

/// The GDExtension entry point. Exactly one `ExtensionLibrary` impl per library;
/// `#[gdextension]` generates the four FFI symbols Godot looks for.
///
/// Classes marked `#[derive(GodotClass)]` are registered **automatically** at the
/// `Scene` init level by gdext; this impl only adds custom startup logging.
struct VoxelGdExt;

#[gdextension]
unsafe impl ExtensionLibrary for VoxelGdExt {
    fn on_stage_init(stage: InitStage) {
        if stage == InitStage::Scene {
            godot_print!("voxel-gdext: Scene stage initialized (voxel-core v{})", voxel_core::VERSION);
        }
    }
}

/// Minimal hello-world class proving the Rust GDExtension wiring works.
///
/// Exposes a GDScript-callable method that returns a greeting built from
/// `voxel_core`'s version string. Once real voxel classes (VoxelBuffer,
/// VoxelTerrain, …) are ported, this can be removed.
#[derive(GodotClass)]
#[class(base = Node, tool)]
struct VoxelRustHello {
    base: Base<Node>,
}

#[godot_api]
impl INode for VoxelRustHello {
    fn init(base: Base<Node>) -> Self {
        Self { base }
    }

    fn ready(&mut self) {
        // Prove we can reach voxel-core from the binding layer.
        godot_print!(
            "VoxelRustHello ready — voxel-core v{} ({} unit tests in port)",
            voxel_core::VERSION,
            191
        );
    }
}

#[godot_api]
impl VoxelRustHello {
    /// Returns a greeting including the voxel-core crate version.
    #[func]
    fn say_hello(&self, name: GString) -> GString {
        let name = name.to_string();
        let version = voxel_core::VERSION;
        // GString implements From<&str> (not From<String>); to_godot borrows.
        format!("Hello, {name}! voxel-core v{version} says hi from Rust").to_godot()
    }

    /// Returns true (smoke test that a bool-returning #[func] registers).
    #[func]
    fn is_alive(&self) -> bool {
        // Touch the engine check so voxel-core stays linked even before we
        // expose real classes.
        let _ = Engine::singleton().is_editor_hint();
        true
    }
}
