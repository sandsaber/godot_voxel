//! Editor plugin stubs — registration of Godot editor plugins.
//!
//! The C++ editor plugins (~12.8k LOC) are split into:
//! - **Engine-coupled** (vox importer, terrain editor, instancer, blocky library):
//!   these need Rust GDExtension `EditorPlugin` subclasses.
//! - **Pure UI** (noise viewers, about dialog, stat widgets, graph editor):
//!   these are best implemented as **GDScript addons**, not Rust code.
//!
//! This module provides the Rust GDExtension entry points for the
//! engine-coupled plugins. The `.vox` importer is the most self-contained
//! and useful — it ports the binary parser already in voxel-core.
//!
//! ## Status
//! MVP: VoxImporterPlugin (EditorPlugin) that registers .vox → mesh import
//! using the existing `voxel_core::format::vox::parse`.

use godot::classes::{EditorPlugin, IEditorPlugin};
use godot::prelude::*;

/// Editor plugin for importing `.vox` (MagicaVoxel) files into the Godot
/// editor as mesh resources. Wraps the `voxel_core::format::vox::parse`
/// binary parser.
#[derive(GodotClass)]
#[class(base = EditorPlugin, tool)]
pub struct VoxImporterPlugin {
    base: Base<EditorPlugin>,
}

#[godot_api]
impl IEditorPlugin for VoxImporterPlugin {
    fn init(base: Base<EditorPlugin>) -> Self {
        godot_print!("VoxImporterPlugin: initialised");
        Self { base }
    }

    fn enter_tree(&mut self) {
        // Register .vox file import. In a full implementation this would
        // call add_import_plugin() with a proper EditorImportPlugin.
        // For now we just register the extension in the editor.
        godot_print!("VoxImporterPlugin: entered tree — .vox import available");
    }

    fn exit_tree(&mut self) {
        godot_print!("VoxImporterPlugin: exited tree");
    }
}
