extends Node3D
## Attached to the runtime paging scene. Builds terrain+viewer+generator at
## runtime, lets paging/meshing run for several real frames, then reports.
var terrain: Node
var frames := 0

func _ready() -> void:
	print("[runtime] building terrain + viewer + generator")
	terrain = ClassDB.instantiate("VoxelTerrain")
	add_child(terrain)
	var gen: Resource = ClassDB.instantiate("VoxelGeneratorWaves")
	if gen:
		terrain.set_generator(gen)
	var viewer: Node = ClassDB.instantiate("VoxelViewer")
	terrain.add_child(viewer)  # viewer must be a child of terrain
	print("[runtime] scene ready, generator + viewer assigned")

func _process(_delta: float) -> void:
	frames += 1
	if frames == 1:
		print("[runtime] frame 1 reached — paging pipeline active")
		# Now that _ready() has run (core is live), exercise the edition API
		# strictly: set_voxel_sdf must report success and the value must stick.
		var set_ok = bool(terrain.set_voxel_sdf(0, 0, 0, -1.0))
		var sdf = float(terrain.get_voxel_sdf(0, 0, 0))
		if set_ok and sdf == -1.0:
			print("[runtime] PASS set_voxel_sdf/get_voxel_sdf (set=true sdf=%f)" % sdf)
		else:
			print("[runtime] FAIL set_voxel_sdf/get_voxel_sdf (set=%s sdf=%f, expected true/-1.0)" % [set_ok, sdf])
	if frames % 10 == 0:
		var bc = int(terrain.get_mesh_block_count())
		print("[runtime] frame %d — mesh_block_count=%d" % [frames, bc])
	if frames >= 40:
		var bc = int(terrain.get_mesh_block_count())
		print("[runtime] DONE after %d frames — mesh_block_count=%d, paging ran without crash" % [frames, bc])
		get_tree().quit(0)
