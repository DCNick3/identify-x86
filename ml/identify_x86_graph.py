from torch_geometric.data import Data
from numpy.compat import os_fspath
import numpy as np
import torch

# monkey patch numpy to support zstd for npz
def zipfile_factory(file, *args, **kwargs):
    """
    Create a ZipFile.
    Allows for Zip64, and the `file` argument can accept file, str, or
    pathlib.Path objects. `args` and `kwargs` are passed to the zipfile.ZipFile
    constructor.
    """
    if not hasattr(file, 'read'):
        file = os_fspath(file)
    import zipfile_zstd as zipfile
    kwargs['allowZip64'] = True
    return zipfile.ZipFile(file, *args, **kwargs)
np.lib.npyio.zipfile_factory = zipfile_factory

def load_graph(filename, name=None):
    npz = np.load(filename)
    
    G = Data()
    G.name = name
    G.num_nodes = npz['instruction_codes'].shape[0]
    G.x_code = torch.from_numpy(npz['instruction_codes']).to(torch.long)
    G.x_size = torch.from_numpy(npz['instruction_sizes']).to(torch.long)
    G.y = torch.from_numpy(npz['instruction_labels']).to(torch.long)

    edge_idx = torch.from_numpy(npz['relations']).to(torch.long)
    edge_ty = torch.from_numpy(npz['relation_types'])

    G.num_edges = edge_idx.shape[0]
    G.edge_index = torch.swapaxes(edge_idx, 0, 1)
    G.edge_type = edge_ty

    return G

def load_split(filename):
    def gen():
        with open(filename) as f:
            for line in f.read().splitlines():
                if line.strip() == '':
                    continue
                yield tuple(line.split(' ', 1)[::-1])
    return dict(gen())