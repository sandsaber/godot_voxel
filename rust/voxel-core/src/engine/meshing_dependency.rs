//! Meshing lifetime dependency ported from `engine/meshing_dependency.h`.

use crate::meshers::SharedVoxelMesher;
use crate::storage::SharedVoxelGenerator;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Shared dependency needed by mesh block tasks.
///
/// Ported from C++ `MeshingDependency`. Holds the mesher and generator the
/// terrain currently uses; if the user swaps either one we build a fresh
/// [`MeshingDependency`] and mark every previously-handed-out reference
/// invalid, rather than mutating in place (the C++ comment explains this is
/// cheaper than fine-grained mutexes given how rarely these swap).
///
pub struct MeshingDependency {
    mesher: SharedVoxelMesher,
    generator: Option<SharedVoxelGenerator>,
    valid: AtomicBool,
}

impl MeshingDependency {
    pub fn new(mesher: SharedVoxelMesher, generator: Option<SharedVoxelGenerator>) -> Arc<Self> {
        Arc::new(Self {
            mesher,
            generator,
            valid: AtomicBool::new(true),
        })
    }

    /// Invalidates any previous dependency stored in `slot`, then installs a
    /// fresh one. Mirrors `MeshingDependency::reset`.
    pub fn reset(
        slot: &mut Option<Arc<Self>>,
        mesher: SharedVoxelMesher,
        generator: Option<SharedVoxelGenerator>,
    ) -> Arc<Self> {
        if let Some(previous) = slot.take() {
            previous.invalidate();
        }
        let dependency = Self::new(mesher, generator);
        *slot = Some(dependency.clone());
        dependency
    }

    pub fn mesher(&self) -> SharedVoxelMesher {
        self.mesher.clone()
    }

    pub fn generator(&self) -> Option<SharedVoxelGenerator> {
        self.generator.clone()
    }

    pub fn is_valid(&self) -> bool {
        self.valid.load(Ordering::Acquire)
    }

    pub fn invalidate(&self) {
        self.valid.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::MeshingDependency;
    use crate::generators::base::{GenResult, VoxelGenerator, VoxelQueryData};
    use crate::meshers::{MesherInput, MesherOutput, VoxelMesher};
    use crate::storage::SharedVoxelGenerator;
    use std::sync::Arc;

    struct NoOpMesher;
    impl VoxelMesher for NoOpMesher {
        fn build(&self, _output: &mut MesherOutput, _input: &MesherInput<'_>) {}
    }

    struct NoOpGenerator;
    impl VoxelGenerator for NoOpGenerator {
        fn generate_block(&self, _input: VoxelQueryData<'_>) -> GenResult {
            GenResult::default()
        }
    }

    fn mesher_handle() -> Arc<dyn VoxelMesher> {
        Arc::new(NoOpMesher)
    }

    fn generator_handle() -> SharedVoxelGenerator {
        Arc::new(NoOpGenerator)
    }

    #[test]
    fn reset_invalidates_previous_dependency() {
        let mut slot = None;
        let mesher = mesher_handle();
        let gen = generator_handle();

        let first = MeshingDependency::reset(&mut slot, mesher.clone(), Some(gen.clone()));
        assert!(first.is_valid());
        assert!(Arc::ptr_eq(&first.mesher(), &mesher));
        assert!(first.generator().is_some());

        let second = MeshingDependency::reset(&mut slot, mesher.clone(), None);
        assert!(!first.is_valid());
        assert!(second.is_valid());
        assert!(Arc::ptr_eq(&second.mesher(), &mesher));
        assert!(second.generator().is_none());
        assert!(Arc::ptr_eq(slot.as_ref().unwrap(), &second));
    }

    #[test]
    fn dependency_can_be_shared_as_arc() {
        let mesher = mesher_handle();
        let dependency = MeshingDependency::new(mesher, None);
        let cloned = dependency.clone();
        // Both handles report the same validity and reach the same mesher.
        assert_eq!(dependency.is_valid(), cloned.is_valid());
        assert!(Arc::ptr_eq(&dependency.mesher(), &cloned.mesher()));
        cloned.invalidate();
        assert!(!dependency.is_valid());
    }
}
