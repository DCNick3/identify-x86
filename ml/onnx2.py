import torch
import torch_geometric

import identify_x86_graph
import identify_x86_model

print("Torch v" + torch.__version__)

model = identify_x86_model.LightningModel.load_from_checkpoint('lightning_logs/version_51/checkpoints/epoch=4-step=4345.ckpt')

G = identify_x86_graph.load_graph('data/raw/debian/buster/binutils-i686-linux-gnu/usr_bin_i686-linux-gnu-addr2line.graph')

example_input = (G.x_code, G.x_size, G.edge_index, G.edge_type)

model_jit = torch.jit.script(model.model)
print("Model JIT:", model_jit)

with torch.no_grad():
    import pdb; pdb.set_trace()
    example_output = model_jit(*example_input)

print(example_output)