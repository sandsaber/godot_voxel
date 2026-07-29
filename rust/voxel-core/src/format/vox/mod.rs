//! `format::vox` — MagicaVoxel `.vox` file format reader.
//!
//! Pure-data parser for the MagicaVoxel binary format. Reads a `.vox` byte
//! stream into a [`data::Data`] tree of models, scene-graph nodes, layers and
//! materials plus the 256-color palette. No Godot dependency; the only I/O is
//! a `&[u8]` cursor.
//!
//! Ported from `streams/vox/{vox_data.h,vox_data.cpp}`. The Godot-facing
//! `vox_loader.cpp` shim (a `RefCounted` writing into `VoxelBuffer`) is
//! intentionally out of scope — it lands with the `voxel-gdext` binding layer.
//!
//! # Example
//! ```no_run
//! use voxel_core::format::vox;
//!
//! let bytes = std::fs::read("scene.vox").unwrap();
//! let data = vox::parse(&bytes).expect("invalid vox file");
//! println!("{} model(s)", data.model_count());
//! ```

pub mod data;
mod parser;

pub use data::{
    Data, GroupNode, Layer, Material, MaterialType, Model, Node, NodeCommon, Rotation, ShapeNode,
    TransformNode, MAX_MODEL_SIZE, PALETTE_SIZE,
};
pub use parser::{parse, parse_with_limits, VoxError};

#[cfg(test)]
mod tests;
