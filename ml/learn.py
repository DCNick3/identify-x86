#!/usr/bin/env python
# coding: utf-8

# In[1]:


#!g1.1
# %pip install -U plotly==5.11.0 pandas==1.3.5 pytorch_lightning==1.8.0 numpy==1.21.6 ipywidgets==8.0.2 
# %pip install torch-scatter torch-sparse torch-cluster torch-spline-conv torch-geometric -f https://data.pyg.org/whl/torch-1.12.0+cu116.html
# %pip install tensorboard==2.11.0


# In[2]:


#!g1.1
import plotly.express as px
import pandas as pd
import torch
import pytorch_lightning as pl
import numpy as np
import pickle
import torchtext
import torch_geometric
from tqdm import tqdm
import identify_x86_data


# ## Dataset loading

# In[3]:


#!g1.1

# EXECUTABLE = 'bin_zsh5'
EXECUTABLE = 'bin_gzip'
# EXECUTABLE = 'usr_lib_gcc_i686-linux-gnu_8_lto1'

df = pd.read_parquet(f'{EXECUTABLE}.superset')
df.code = df.code.map(lambda x: identify_x86_data.INSTR_CODES[x])
df.set_index('addr', inplace=True)
df.sort_index(inplace=True)
df


# In[4]:


#!g1.1
df


# In[5]:


#!g1.1
# https://wiki.osdev.org/X86-64_Instruction_Encoding#General_Overview (x86 should be smaller but meh)
MAX_ISN_SIZE = 15

df['size'].value_counts()


# In[6]:


#!g1.1
df.code.value_counts()


# In[7]:


#!g1.1
from torchtext.vocab import vocab as make_vocab
from collections import OrderedDict

# TODO: this vocab should be built over the whole training dataset

counts = df.code.value_counts()

known = { x: i for i, x in enumerate(counts.index[:200]) }
vocab = make_vocab(known, specials=['INVALID', 'UNKNOWN'])
vocab.set_default_index(vocab['UNKNOWN'])


# In[8]:


#!g1.1
vocab['Dec_r32']


# ## Build graph from the loaded data

# In[9]:


#!g1.1
from torch_geometric.data import Data
from typing import List, Tuple
import gc

def encode_instructions(instr):
    code = torch.tensor(vocab(instr.code.to_list()))
    size = torch.tensor(instr['size'].map(lambda x: x-1).values)
    labels = torch.tensor(instr['label'].values)

    return code, size, labels

EDGE_NEXT = 0
EDGE_PREV = 1
EDGE_OVERLAP = 2
EDGE_RELCOUNT = 3

class EdgesBuilder:
    def __init__(self):
        self.idx_buffer = []
        self.ty_buffer = []
        self.edge_count = 0
        self.edge_idx_parts = []
        self.edge_ty_parts = []

    def add_edge(self, src, dst, kind):
        self.idx_buffer.append((src, dst))
        self.ty_buffer.append(kind)
        self.edge_count += 1
        
        if len(self.idx_buffer) >= 0x80000: # TODO: tune
            self.edge_idx_parts.append(torch.tensor(self.idx_buffer, dtype=torch.long))
            self.edge_ty_parts.append(torch.tensor(self.ty_buffer, dtype=torch.long))
            self.idx_buffer.clear()
            self.ty_buffer.clear()

    def build(self):
        self.edge_idx_parts.append(torch.tensor(self.idx_buffer, dtype=torch.long))
        self.edge_ty_parts.append(torch.tensor(self.ty_buffer, dtype=torch.long))
        self.idx_buffer.clear()
        self.ty_buffer.clear()

        edge_idx = torch.cat(self.edge_idx_parts)
        self.edge_idx_parts.clear()
        gc.collect()
        
        edge_ty = torch.cat(self.edge_ty_parts)
        self.edge_ty_parts.clear()
        gc.collect()

        return edge_idx, edge_ty

    def __len__(self):
        return self.edge_count

def build_executable_graph(df):
    G = Data()
    G.num_nodes = df.shape[0]
    G.x_code, G.x_size, G.y = encode_instructions(df)
    # the classes are stored as booleans for efficiency
    # convert them to long for the loss function
    G.y = G.y.to(torch.long)

    edges = EdgesBuilder()

    t = tqdm(df.iterrows(), total=df.shape[0])
    for addr, x in t:
        i = df.index.get_loc(addr)
        next_addr = addr + x.size
        try:
            j = df.index.get_loc(next_addr)

            edges.add_edge(i, j, EDGE_NEXT)
            edges.add_edge(j, i, EDGE_PREV)
        except KeyError:
            pass

        for o in range(addr+1, next_addr):
            try:
                j = df.index.get_loc(o)
                edges.add_edge(i, j, EDGE_OVERLAP)
                edges.add_edge(j, i, EDGE_OVERLAP)
            except KeyError:
                pass
        if addr % 0x1000 == 0:
            t.set_description(f'edges: {len(edges)}')

    edge_idx, edge_ty = edges.build()
    del edges
    gc.collect()

    print(edge_idx.shape)
    print(edge_idx)

    print(edge_ty)

    G.num_edges = edge_idx.shape[0]
    G.edge_index = torch.swapaxes(edge_idx, 0, 1)
    G.edge_type = edge_ty

    return G


# In[10]:


#!g1.1
# TODO: use PyG's Dataset class
G = build_executable_graph(df)


# In[11]:


torch.save(G, f'{EXECUTABLE}.graph')


# In[12]:


#!g1.1
from torch_geometric.nn import Sequential, RGCNConv, Linear
from torch.nn import Embedding, ReLU, Sigmoid
import torchmetrics
import torchmetrics.classification


# In[13]:


#!g1.1

from torch import Tensor

class IdentifyModel(torch.nn.Module):
    def __init__(self) -> None:
        super().__init__()

        size_embed_size = 4
        code_embed_size = 32

        @torch.jit.script
        def cat(x1, x2):
            return torch.cat([x1, x2], dim=1)

        self.model = Sequential('x_code, x_size, edge_index, edge_type', [
            (Embedding(num_embeddings=MAX_ISN_SIZE, embedding_dim=size_embed_size), 'x_size -> x_size'),
            (Embedding(num_embeddings=len(vocab), embedding_dim=code_embed_size), 'x_code -> x_code'),
            (cat, 'x_size, x_code -> x'),
            (RGCNConv(size_embed_size + code_embed_size, 24, EDGE_RELCOUNT).jittable(), 'x, edge_index, edge_type -> x'),
            ReLU(inplace=True),
            (RGCNConv(24, 16, EDGE_RELCOUNT).jittable(), 'x, edge_index, edge_type -> x'),
            ReLU(inplace=True),
            (RGCNConv(16, 8, EDGE_RELCOUNT).jittable(), 'x, edge_index, edge_type -> x'),
            ReLU(inplace=True),
            (RGCNConv(8, 4, EDGE_RELCOUNT).jittable(), 'x, edge_index, edge_type -> x'),
            ReLU(inplace=True),
            Linear(4, 2),
        ])
    
    def forward(self, x_code: Tensor, x_size: Tensor, edge_index: Tensor, edge_type: Tensor) -> Tensor:
        return self.model(x_code, x_size, edge_index, edge_type)


class LightningModel(pl.LightningModule):
    def __init__(self):
        super(LightningModel, self).__init__()

        self.model = IdentifyModel()

        self.train_accuracy = torchmetrics.classification.BinaryAccuracy()
        self.train_precision = torchmetrics.classification.BinaryPrecision()
        self.train_recall = torchmetrics.classification.BinaryRecall()

        self.valid_accuracy = torchmetrics.classification.BinaryAccuracy()
        self.valid_precision = torchmetrics.classification.BinaryPrecision()
        self.valid_recall = torchmetrics.classification.BinaryRecall()


    def forward(self, x_code: Tensor, x_size: Tensor, edge_index: Tensor, edge_type: Tensor):
        x_out = self.model(x_code, x_size, edge_index, edge_type)

        return x_out

    def training_step(self, batch, batch_index):
        x_code, x_size, edge_index, edge_type =             batch.x_code, batch.x_size, batch.edge_index, batch.edge_type

        x_out = self.forward(x_code, x_size, edge_index, edge_type)

        loss = torch.nn.functional.cross_entropy(x_out, batch.y)

        # metrics here
        pred = x_out.argmax(-1)
        label = batch.y
        
        self.train_accuracy(pred, label)
        self.train_precision(pred, label)
        self.train_recall(pred, label)

        self.log("loss/train", loss)
        self.log("accuracy/train", self.train_accuracy, on_step=True, on_epoch=False)
        self.log("recall/train", self.train_recall, on_step=True, on_epoch=False)
        self.log("precision/train", self.train_precision, on_step=True, on_epoch=False)

        return loss

    def validation_step(self, batch, batch_index):
        x_code, x_size, edge_index, edge_type =             batch.x_code, batch.x_size, batch.edge_index, batch.edge_type

        x_out = self.forward(x_code, x_size, edge_index, edge_type)

        #loss = torch.nn.functional.cross_entropy(x_out, batch.y)

        pred = x_out.argmax(-1)

        self.valid_accuracy(pred, batch.y)
        self.valid_precision(pred, batch.y)
        self.valid_recall(pred, batch.y)

        self.log("accuracy/val", self.valid_accuracy, on_step=True, on_epoch=True)
        self.log("recall/val", self.valid_recall, on_step=True, on_epoch=True)
        self.log("precision/val", self.valid_precision, on_step=True, on_epoch=True)

        return x_out, pred, batch.y

    def validation_epoch_end(self, validation_step_outputs):
        val_loss = 0.0
        num_correct = 0
        num_total = 0
        num_tp = 0
        num_tn = 0
        num_fp = 0
        num_fn = 0

        for output, pred, labels in validation_step_outputs:
            val_loss += torch.nn.functional.cross_entropy(output, labels, reduction="sum")

        self.log("loss/val", val_loss)

    def configure_optimizers(self):
        return torch.optim.Adam(self.parameters(), lr = 3e-4)


# In[14]:


#!g1.1
model = LightningModel()


# In[15]:


#!g1.1
model


# In[16]:


#!g1.1
from torch.utils.data import DataLoader
from pytorch_lightning.loggers import TensorBoardLogger

print("Cuda is available:", torch.cuda.is_available())

# TODO: ehh... we want more than one graph, right?)
dataset = [G]

train_loader = DataLoader(dataset, batch_size=None)
test_loader = DataLoader(dataset, batch_size=None)
val_loader = DataLoader(dataset, batch_size=None)

model = LightningModel()
num_epochs = 10
# val_check_interval = len(train_loader)

trainer = pl.Trainer(
    max_epochs = num_epochs,
    # val_check_interval = val_check_interval,
    log_every_n_steps = 1,
    accelerator = 'cpu',
    enable_progress_bar = False,
)
trainer.fit(model, train_loader, val_loader)


# In[ ]:


#!g1.1
# TODO: this is not that easy...
# model.eval()
# for param in model.parameters():
    # param.requires_grad = False
model_jit = torch.jit.script(model.model)
print("Model JIT:", model_jit)
torch.onnx.export(model_jit, (
    torch.tensor([0], dtype=torch.long),
    torch.tensor([0], dtype=torch.long),
    torch.tensor([[0, 0]], dtype=torch.long),
    torch.tensor([0], dtype=torch.long),
), f"{EXECUTABLE}.onnx", verbose=True, opset_version=16)
# with torch.no_grad():

    # torch.onnx.export(model, (G.x_code, G.x_size, G.edge_index, G.edge_type), f"{EXECUTABLE}.onnx", verbose=True)
# torch.jit.save(model, f"{EXECUTABLE}.pt")


# In[ ]:


model.model[-1].weight


# In[ ]:




