import argparse
from pathlib import Path
import sys

import torch
import torch_geometric

import identify_x86_model

def eprint(*args, **kwargs):
    print(*args, file=sys.stderr, **kwargs)

eprint("Torch v" + torch.__version__)

parser = argparse.ArgumentParser(description='Convert a model checkpoint to a standalone TorchScript module')
parser.add_argument('checkpoint', type=Path, help='Path to model checkpoint file')
parser.add_argument('output', type=Path, help='Path to output TorchScript file')

args = parser.parse_args()

model = identify_x86_model.LightningModel.load_from_checkpoint(args.checkpoint)

model_jit = torch.jit.script(model.model)
eprint("Model JIT:", model_jit)

model_jit.save(args.output)
