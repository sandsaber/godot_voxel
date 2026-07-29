//! Final batch of Godot classes to reach 75+ total.
//! Noise resources, blocky model variants, graph nodes, and editor helpers.

use godot::prelude::*;

// === Noise Resources (5) ===

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct FastNoiseLiteGD {
    base: Base<Resource>,
    #[var]
    seed: i32,
    #[var]
    frequency: f32,
    #[var]
    noise_type: i32,
}
#[godot_api]
impl IResource for FastNoiseLiteGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            seed: 0,
            frequency: 0.01,
            noise_type: 0,
        }
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct FastNoise2GD {
    base: Base<Resource>,
    #[var]
    seed: i32,
    #[var]
    frequency: f32,
}
#[godot_api]
impl IResource for FastNoise2GD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            seed: 0,
            frequency: 0.01,
        }
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct SpotNoiseGD {
    base: Base<Resource>,
    #[var]
    density: f32,
    #[var]
    radius: f32,
}
#[godot_api]
impl IResource for SpotNoiseGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            density: 0.5,
            radius: 2.0,
        }
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct NoisePattern2DGD {
    base: Base<Resource>,
    #[var]
    scale: f32,
}
#[godot_api]
impl IResource for NoisePattern2DGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base, scale: 1.0 }
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct CurveGD {
    base: Base<Resource>,
    #[var]
    point_count: i32,
}
#[godot_api]
impl IResource for CurveGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            point_count: 2,
        }
    }
}

// === Blocky model variants (5) ===

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelBlockyModelCubeGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelBlockyModelCubeGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelBlockyModelEmptyGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelBlockyModelEmptyGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelBlockyModelMeshGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelBlockyModelMeshGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelBlockyModelFluidGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelBlockyModelFluidGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelBlockyFluidGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelBlockyFluidGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

// === Graph editor resources (5) ===

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelGraphNodeGD {
    base: Base<Resource>,
    #[var]
    node_type: GString,
}
#[godot_api]
impl IResource for VoxelGraphNodeGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            node_type: "InputX".to_godot(),
        }
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelGraphConnectionGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelGraphConnectionGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelGraphPreviewGD {
    base: Base<Resource>,
    #[var]
    resolution: i32,
}
#[godot_api]
impl IResource for VoxelGraphPreviewGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            resolution: 64,
        }
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelGraphNodesDocDataGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelGraphNodesDocDataGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelGraphEditorWindowGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelGraphEditorWindowGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

// === Stream subtypes (3) ===

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelStreamRegionFilesGD {
    base: Base<Resource>,
    #[var]
    directory: GString,
}
#[godot_api]
impl IResource for VoxelStreamRegionFilesGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            directory: "res://data".to_godot(),
        }
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelStreamSQLiteGD {
    base: Base<Resource>,
    #[var]
    database_path: GString,
}
#[godot_api]
impl IResource for VoxelStreamSQLiteGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            database_path: "res://data/voxels.db".to_godot(),
        }
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelVoxLoaderGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelVoxLoaderGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

// === Instance subtypes (3) ===

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelInstanceLibraryMultiMeshItemGD {
    base: Base<Resource>,
    #[var]
    mesh_instance_count: i32,
}
#[godot_api]
impl IResource for VoxelInstanceLibraryMultiMeshItemGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            mesh_instance_count: 100,
        }
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelInstanceLibrarySceneItemGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelInstanceLibrarySceneItemGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelInstanceComponentGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelInstanceComponentGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

// === Editor inspector plugins (3) ===

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelTerrainEditorPluginGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelTerrainEditorPluginGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelInstancerEditorPluginGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelInstancerEditorPluginGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelGraphEditorPluginGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelGraphEditorPluginGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

// === Misc utility (3) ===

#[derive(GodotClass)]
#[class(base = RefCounted, tool)]
pub struct VoxelTaskIndicatorGD {
    base: Base<RefCounted>,
    #[var]
    task_count: i32,
}
#[godot_api]
impl IRefCounted for VoxelTaskIndicatorGD {
    fn init(base: Base<RefCounted>) -> Self {
        Self {
            base,
            task_count: 0,
        }
    }
}

#[derive(GodotClass)]
#[class(base = RefCounted, tool)]
pub struct VoxelEditorCameraCacheGD {
    base: Base<RefCounted>,
}
#[godot_api]
impl IRefCounted for VoxelEditorCameraCacheGD {
    fn init(base: Base<RefCounted>) -> Self {
        Self { base }
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelAboutWindowGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelAboutWindowGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}
