@tool
extends Control

## Graph editor panel — a GraphEdit-based visual editor for VoxelGeneratorGraph.
##
## Provides node creation, connection, and parameter editing through the
## VoxelGeneratorGraphGD Rust API (set_graph_json / get_graph_json /
## compile_and_sample / get_node_count).

var _graph_edit: GraphEdit
var _current_graph: VoxelGeneratorGraphGD
var _toolbar: HBoxContainer
var _add_node_menu: MenuButton
var _status_label: Label

func _ready() -> void:
	_build_ui()

func _build_ui() -> void:
	# Layout: VBoxContainer with toolbar on top, GraphEdit below.
	var vbox := VBoxContainer.new()
	vbox.set_anchors_preset(PRESET_FULL_RECT)
	add_child(vbox)

	# Toolbar
	_toolbar = HBoxContainer.new()
	vbox.add_child(_toolbar)

	_add_node_menu = MenuButton.new()
	_add_node_menu.text = "Add Node"
	var popup := _add_node_menu.get_popup()
	popup.add_item("Input X", 0)
	popup.add_item("Input Y", 1)
	popup.add_item("Input Z", 2)
	popup.add_item("Constant", 3)
	popup.add_item("Output SDF", 4)
	popup.add_item("SDF Sphere", 5)
	popup.add_item("SDF Plane", 6)
	popup.add_item("Add", 7)
	popup.add_item("Multiply", 8)
	popup.add_item("Expression", 9)
	popup.id_pressed.connect(_on_add_node)
	_toolbar.add_child(_add_node_menu)

	var compile_btn := Button.new()
	compile_btn.text = "Compile & Sample"
	compile_btn.pressed.connect(_on_compile)
	_toolbar.add_child(compile_btn)

	var clear_btn := Button.new()
	clear_btn.text = "Clear"
	clear_btn.pressed.connect(_on_clear)
	_toolbar.add_child(clear_btn)

	_status_label = Label.new()
	_status_label.text = "No graph selected"
	_toolbar.add_child(_status_label)

	# GraphEdit
	_graph_edit = GraphEdit.new()
	_graph_edit.set_anchors_preset(PRESET_FULL_RECT)
	_graph_edit.connection_request.connect(_on_connection_request)
	_graph_edit.disconnection_request.connect(_on_disconnection_request)
	vbox.add_child(_graph_edit)

func edit_graph(graph: VoxelGeneratorGraphGD) -> void:
	_current_graph = graph
	_status_label.text = "Editing graph (%d nodes)" % graph.get_node_count()
	# Load existing nodes from graph_json
	_refresh_from_graph()

func _refresh_from_graph() -> void:
	_graph_edit.clear_connections()
	for child in _graph_edit.get_children():
		if child is GraphNode:
			child.queue_free()
	if not _current_graph:
		return
	# In a full implementation, we'd parse graph_json and create GraphNode widgets.
	# For now, this is the structural scaffold.

func _on_add_node(id: int) -> void:
	if not _current_graph:
		_status_label.text = "No graph selected"
		return
	# Create a visual GraphNode
	var node_names := {
		0: "InputX", 1: "InputY", 2: "InputZ", 3: "Constant",
		4: "OutputSDF", 5: "SdfSphere", 6: "SdfPlane",
		7: "Add", 8: "Multiply", 9: "Expression",
	}
	var gn := GraphNode.new()
	gn.title = node_names.get(id, "Node")
	gn.position_offset = Vector2(100 + randf() * 200, 100 + randf() * 100)
	# Add a label as content
	var label := Label.new()
	label.text = "Type: " + gn.title
	gn.add_child(label)
	# Add ports
	gn.set_slot(0, true, 0, Color(0.5, 0.8, 0.5), true, 0, Color(0.8, 0.5, 0.5))
	_graph_edit.add_child(gn)
	_status_label.text = "Added: %s (total %d)" % [gn.title, _current_graph.get_node_count()]

func _on_connection_request(from_node: StringName, from_port: int, to_node: StringName, to_port: int) -> void:
	_graph_edit.connect_node(from_node, from_port, to_node, to_port)

func _on_disconnection_request(from_node: StringName, from_port: int, to_node: StringName, to_port: int) -> void:
	_graph_edit.disconnect_node(from_node, from_port, to_node, to_port)

func _on_compile() -> void:
	if not _current_graph:
		_status_label.text = "No graph"
		return
	# Use the Rust API to compile and sample at origin
	var val := _current_graph.compile_and_sample(0.0, 0.0, 0.0)
	_status_label.text = "SDF at origin: %.4f (nodes: %d)" % [val, _current_graph.get_node_count()]

func _on_clear() -> void:
	_graph_edit.clear_connections()
	for child in _graph_edit.get_children():
		if child is GraphNode:
			child.queue_free()
	_status_label.text = "Cleared"
