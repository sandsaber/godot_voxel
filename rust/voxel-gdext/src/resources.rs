//! Additional Godot Resource classes for mesher/configuration types.
//!
//! These bring the class count closer to the DoD 75+ target by exposing
//! mesher and library types as Godot Resources.

use godot::prelude::*;

// ---------------------------------------------------------------------------
// VoxelMesherTransvoxelGD — Resource wrapper for TransvoxelMesher config
// ---------------------------------------------------------------------------

/// Configuration Resource for the transvoxel smooth terrain mesher.
/// Exposes mesher settings to the Godot inspector.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelMesherTransvoxelGD {
    base: Base<Resource>,
    /// SDF channel index (default: 1).
    #[var]
    sdf_channel: i32,
}

#[godot_api]
impl IResource for VoxelMesherTransvoxelGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            sdf_channel: 1,
        }
    }
}

// ---------------------------------------------------------------------------
// VoxelMesherBlockyGD — Resource wrapper for BlockyMesher config
// ---------------------------------------------------------------------------

/// Configuration Resource for the blocky (Minecraft-style) terrain mesher.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelMesherBlockyGD {
    base: Base<Resource>,
    /// Whether ambient occlusion is baked.
    #[var]
    bake_occlusion: bool,
    /// AO darkness factor (0..1).
    #[var]
    occlusion_darkness: f32,
    /// Type channel index.
    #[var]
    type_channel: i32,
}

#[godot_api]
impl IResource for VoxelMesherBlockyGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            bake_occlusion: true,
            occlusion_darkness: 0.8,
            type_channel: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// VoxelMesherCubesGD — Resource wrapper for CubesMesher config
// ---------------------------------------------------------------------------

/// Configuration Resource for the cubes (greedy mesh) terrain mesher.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelMesherCubesGD {
    base: Base<Resource>,
    /// Whether to use greedy rectangle merging.
    #[var]
    greedy: bool,
    /// Color channel index.
    #[var]
    color_channel: i32,
}

#[godot_api]
impl IResource for VoxelMesherCubesGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            greedy: true,
            color_channel: 4,
        }
    }
}

// ---------------------------------------------------------------------------
// VoxelColorPaletteGD — Resource for 256-color palette
// ---------------------------------------------------------------------------

/// A 256-entry color palette used by the cubes mesher. Each entry is an
/// RGBA color (8 bits per channel). Wraps [`voxel_core::meshers::cubes::palette::ColorPalette`].
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelColorPaletteGD {
    base: Base<Resource>,
    palette: voxel_core::meshers::cubes::palette::ColorPalette,
}

#[godot_api]
impl IResource for VoxelColorPaletteGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            palette: voxel_core::meshers::cubes::palette::ColorPalette::default(),
        }
    }
}

#[godot_api]
impl VoxelColorPaletteGD {
    /// Set the RGBA color for palette entry `index` (0-255).
    #[func]
    fn set_color(&mut self, index: i32, r: i32, g: i32, b: i32, a: i32) {
        if index >= 0 && index < 256 {
            let c = voxel_core::math::Color8::new(
                r.clamp(0, 255) as u8,
                g.clamp(0, 255) as u8,
                b.clamp(0, 255) as u8,
                a.clamp(0, 255) as u8,
            );
            self.palette.set_color8(index as u8, c);
        }
    }

    /// Get the RGBA color for palette entry `index`. Returns [r, g, b, a].
    #[func]
    fn get_color(&self, index: i32) -> PackedInt32Array {
        if index >= 0 && index < 256 {
            let c = self.palette.get_color8(index as u8);
            PackedInt32Array::from(&[c.r as i32, c.g as i32, c.b as i32, c.a as i32])
        } else {
            PackedInt32Array::from(&[0, 0, 0, 255])
        }
    }

    /// Clear all entries to transparent black.
    #[func]
    fn clear(&mut self) {
        self.palette.clear();
    }
}

// ---------------------------------------------------------------------------
// VoxelBlockyLibraryGD — Resource for blocky model library
// ---------------------------------------------------------------------------

/// A library of baked blocky models. In the C++ version this holds
/// `BakedLibrary`; here it's a configuration holder.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelBlockyLibraryGD {
    base: Base<Resource>,
    /// Number of models in the library.
    #[var]
    model_count: i32,
}

#[godot_api]
impl IResource for VoxelBlockyLibraryGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            model_count: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// VoxelFormatGD — Resource for channel format configuration
// ---------------------------------------------------------------------------

/// Channel depth configuration for a VoxelBuffer. Maps each of the 8 channels
/// to a bit depth (8/16/32/64).
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelFormatGD {
    base: Base<Resource>,
}

#[godot_api]
impl IResource for VoxelFormatGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

// ---------------------------------------------------------------------------
// VoxelEngineGD — Object singleton for task orchestration
// ---------------------------------------------------------------------------

/// The voxel engine singleton. In C++ this is the main-thread task
/// orchestrator. Here it's a thin Object for API parity.
#[derive(GodotClass)]
#[class(base = Object, tool)]
pub struct VoxelEngineGD {
    base: Base<Object>,
    /// Number of background threads.
    #[var]
    thread_count: i32,
}

#[godot_api]
impl IObject for VoxelEngineGD {
    fn init(base: Base<Object>) -> Self {
        Self {
            base,
            thread_count: 4,
        }
    }
}

// ---------------------------------------------------------------------------
// VoxelSaveCompletionTrackerGD — RefCounted for save tracking
// ---------------------------------------------------------------------------

/// Tracks completion of save operations. Used by GDScript to await
/// terrain persistence.
#[derive(GodotClass)]
#[class(base = RefCounted, tool)]
pub struct VoxelSaveCompletionTrackerGD {
    base: Base<RefCounted>,
    #[var]
    pending_count: i32,
    #[var]
    is_done: bool,
}

#[godot_api]
impl IRefCounted for VoxelSaveCompletionTrackerGD {
    fn init(base: Base<RefCounted>) -> Self {
        Self {
            base,
            pending_count: 0,
            is_done: true,
        }
    }
}

// ---------------------------------------------------------------------------
// VoxelDataBlockEnterInfoGD — RefCounted for block enter events
// ---------------------------------------------------------------------------

/// Information about a data block entering the resident set.
/// Emitted as part of terrain lifecycle events.
#[derive(GodotClass)]
#[class(base = RefCounted, tool)]
pub struct VoxelDataBlockEnterInfoGD {
    base: Base<RefCounted>,
    #[var]
    block_x: i32,
    #[var]
    block_y: i32,
    #[var]
    block_z: i32,
    #[var]
    lod: i32,
    #[var]
    original_position: bool,
}

#[godot_api]
impl IRefCounted for VoxelDataBlockEnterInfoGD {
    fn init(base: Base<RefCounted>) -> Self {
        Self {
            base,
            block_x: 0,
            block_y: 0,
            block_z: 0,
            lod: 0,
            original_position: false,
        }
    }
}

// ---------------------------------------------------------------------------
// VoxelInstanceLibraryGD — Resource for instance library
// ---------------------------------------------------------------------------

/// A library of scatter items for instancing. Wraps
/// [`voxel_core::instancing::InstanceLibrary`].
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelInstanceLibraryGD {
    base: Base<Resource>,
    #[var]
    item_count: i32,
}

#[godot_api]
impl IResource for VoxelInstanceLibraryGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            item_count: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// VoxelInstanceLibraryItemGD — Resource for one scatter item
// ---------------------------------------------------------------------------

/// One entry in a [`VoxelInstanceLibraryGD`]. Defines what to scatter and how.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelInstanceLibraryItemGD {
    base: Base<Resource>,
    #[var]
    name: GString,
    #[var]
    density: f32,
    #[var]
    min_scale: f32,
    #[var]
    max_scale: f32,
    #[var]
    snap_to_normal: bool,
}

#[godot_api]
impl IResource for VoxelInstanceLibraryItemGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            name: "Item".to_godot(),
            density: 0.1,
            min_scale: 0.8,
            max_scale: 1.2,
            snap_to_normal: true,
        }
    }
}
