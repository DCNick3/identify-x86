import torch
import torch_geometric

import identify_x86_graph

print("Torch v" + torch.__version__)

model = torch.jit.load('model_jit2.pt', map_location='cpu')

# print(model)

G = identify_x86_graph.load_graph('data/raw/debian/buster/binutils-i686-linux-gnu/usr_bin_i686-linux-gnu-addr2line.graph')

example_input = (G.x_code, G.x_size, G.edge_index, G.edge_type)

with torch.no_grad():
    # import pdb; pdb.set_trace()
    example_output = model(*example_input)

# print(model.code)

torch.onnx.export(
    model = model,
    args = (G.x_code, G.x_size, G.edge_index, G.edge_type),
    f = 'model.onnx',
    input_names = ['x_code', 'x_size', 'edge_index', 'edge_type'],
    operator_export_type=torch.onnx.OperatorExportTypes.ONNX,
    opset_version = 17,
    verbose = True,
    dynamic_axes = None,
    export_modules_as_functions = False,
)
