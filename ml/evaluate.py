import argparse
from pathlib import Path
import sys

import torch
# import torch_geometric

import identify_x86_graph

def eprint(*args, **kwargs):
    print(*args, file=sys.stderr, **kwargs)

eprint("Using Torch v" + torch.__version__)

parser = argparse.ArgumentParser(description='Run identify-x86 model on a graph')
parser.add_argument('model', type=Path, help='Path to model file (in form of TorchScript module)')
parser.add_argument('graph', type=Path, help='Path to graph file')

args = parser.parse_args()

model = torch.jit.load(args.model, map_location='cpu')

G = identify_x86_graph.load_graph(args.graph)

input = (G.x_code, G.x_size, G.edge_index, G.edge_type)

with torch.no_grad():
    # import pdb; pdb.set_trace()
    output = model(*input)

# print(output)

THRESHOLD = 0.5

for i in range(len(output)):
    r = torch.softmax(output[i], dim=0)
    if r[1] > THRESHOLD:
        print(f"{i}")


# torch.onnx.export(
#     model = model,
#     args = (G.x_code, G.x_size, G.edge_index, G.edge_type),
#     f = 'model.onnx',
#     input_names = ['x_code', 'x_size', 'edge_index', 'edge_type'],
#     operator_export_type=torch.onnx.OperatorExportTypes.ONNX,
#     opset_version = 17,
#     verbose = True,
#     dynamic_axes = None,
#     export_modules_as_functions = False,
# )
