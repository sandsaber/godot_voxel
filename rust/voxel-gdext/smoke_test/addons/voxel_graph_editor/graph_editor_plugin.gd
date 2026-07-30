@tool
extends EditorPlugin

## VoxelGraphEditor plugin — provides a visual GraphEdit-based editor for
## VoxelGeneratorGraph resources. Ports the C++ VoxelGraphEditorPlugin.
##
## Usage:
##   1. Enable this plugin in Project Settings → Plugins.
##   2. Select a VoxelGeneratorGraph resource in the inspector.
##   3. Click "Edit Graph" to open the graph editor bottom panel.

const GraphEditorPanel = preload("res://addons/voxel_graph_editor/graph_editor_panel.gd")

var _panel: Control
var _editor_interface: EditorInterface

func _get_plugin_name() -> String:
	return "VoxelGraphEditor"

func _get_plugin_icon() -> Texture2D:
	# Use a built-in icon as placeholder
	return EditorInterface.get_editor_theme().get_icon("GraphEdit", "EditorIcons")

func _handles(object: Object) -> bool:
	# Show the "Edit Graph" button when a VoxelGeneratorGraph is selected
	return object is VoxelGeneratorGraph

func _make_visible(visible: bool) -> void:
	if _panel:
		_panel.visible = visible

func _enter_tree() -> void:
	_panel = GraphEditorPanel.new()
	_panel.set_custom_minimum_size(Vector2(0, 200))
	add_control_to_bottom_panel(_panel, "Voxel Graph")
	_make_visible(false)
	print("VoxelGraphEditor: plugin entered tree")

func _exit_tree() -> void:
	if _panel:
		remove_control_from_bottom_panel(_panel)
		_panel.queue_free()
		_panel = null
	print("VoxelGraphEditor: plugin exited tree")

func _edit(object: Object) -> void:
	if _panel and object is VoxelGeneratorGraph:
		_panel.edit_graph(object)
