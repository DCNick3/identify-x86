
import torch
from torch import Tensor
from torch_geometric.nn import Sequential, RGCNConv, Linear
from torch.nn import Embedding, ReLU

import pytorch_lightning as pl
import torchmetrics
import torchmetrics.classification

VOCAB_SIZE = 502

class IdentifyModel(torch.nn.Module):
    def __init__(self) -> None:
        super().__init__()

        size_embed_size = 4
        code_embed_size = 32

        @torch.jit.script
        def cat(x1, x2):
            return torch.cat([x1, x2], dim=1)

        from identify_x86_data import RELATION_COUNT, MAX_ISN_SIZE

        self.model = Sequential('x_code, x_size, edge_index, edge_type', [
            (Embedding(num_embeddings=MAX_ISN_SIZE, embedding_dim=size_embed_size), 'x_size -> x_size'),
            (Embedding(num_embeddings=VOCAB_SIZE, embedding_dim=code_embed_size), 'x_code -> x_code'),
            (cat, 'x_size, x_code -> x'),
            (RGCNConv(size_embed_size + code_embed_size, 24, RELATION_COUNT).jittable(), 'x, edge_index, edge_type -> x'),
            ReLU(inplace=True),
            (RGCNConv(24, 16, RELATION_COUNT).jittable(), 'x, edge_index, edge_type -> x'),
            ReLU(inplace=True),
            (RGCNConv(16, 8, RELATION_COUNT).jittable(), 'x, edge_index, edge_type -> x'),
            ReLU(inplace=True),
            (RGCNConv(8, 4, RELATION_COUNT).jittable(), 'x, edge_index, edge_type -> x'),
            ReLU(inplace=True),
            Linear(4, 2),
        ])
    
    def forward(self, x_code: Tensor, x_size: Tensor, edge_index: Tensor, edge_type: Tensor) -> Tensor:
        # print("Forward pass", x_code.shape, edge_type.shape)
        return self.model(x_code, x_size, edge_index, edge_type)


class LightningModel(pl.LightningModule):
    def __init__(self, true_instr_weight: float = 8.0):
        super(LightningModel, self).__init__()

        self.model = IdentifyModel()

        self.loss = torch.nn.CrossEntropyLoss(
            weight = torch.tensor([
                1.0,
                true_instr_weight,
            ])
        )

        self.train_accuracy = torchmetrics.classification.BinaryAccuracy()
        self.train_precision = torchmetrics.classification.BinaryPrecision()
        self.train_recall = torchmetrics.classification.BinaryRecall()
        self.train_f1 = torchmetrics.classification.F1Score(task='binary') # TODO: this argument is from a newer version of torchmetrics

        self.valid_accuracy = torchmetrics.classification.BinaryAccuracy()
        self.valid_precision = torchmetrics.classification.BinaryPrecision()
        self.valid_recall = torchmetrics.classification.BinaryRecall()
        self.valid_f1 = torchmetrics.classification.F1Score(task='binary')

        self.validation_step_outputs = []


    def forward(self, x_code: Tensor, x_size: Tensor, edge_index: Tensor, edge_type: Tensor):
        x_out = self.model(x_code, x_size, edge_index, edge_type)

        return x_out

    def training_step(self, batch, batch_index):
        x_code, x_size, edge_index, edge_type = \
            batch.x_code, batch.x_size, batch.edge_index, batch.edge_type

        x_out = self.forward(x_code, x_size, edge_index, edge_type)

        loss = torch.nn.functional.cross_entropy(x_out, batch.y)

        # metrics here
        pred = x_out.argmax(-1)
        label = batch.y
        
        self.train_accuracy(pred, label)
        self.train_precision(pred, label)
        self.train_recall(pred, label)
        self.train_f1(pred, label)

        self.log("loss/train", loss)
        self.log("accuracy/train", self.train_accuracy, on_step=True, on_epoch=False)
        self.log("recall/train", self.train_recall, on_step=True, on_epoch=False)
        self.log("precision/train", self.train_precision, on_step=True, on_epoch=False)
        self.log("f1/train", self.train_f1, on_step=True, on_epoch=False)

        return loss

    def validation_step(self, batch, batch_index):
        x_code, x_size, edge_index, edge_type = \
            batch.x_code, batch.x_size, batch.edge_index, batch.edge_type

        x_out = self.forward(x_code, x_size, edge_index, edge_type)

        loss = torch.nn.functional.cross_entropy(x_out, batch.y)
        self.validation_step_outputs.append(loss)

        pred = x_out.argmax(-1)

        self.valid_accuracy(pred, batch.y)
        self.valid_precision(pred, batch.y)
        self.valid_recall(pred, batch.y)
        self.valid_f1(pred, batch.y)

        self.log("accuracy/val", self.valid_accuracy, on_step=True, on_epoch=True)
        self.log("recall/val", self.valid_recall, on_step=True, on_epoch=True)
        self.log("precision/val", self.valid_precision, on_step=True, on_epoch=True)
        self.log("f1/val", self.valid_f1, on_step=True, on_epoch=True)

        return x_out, pred, batch.y

    def on_validation_epoch_end(self):
        val_loss = 0.0
        num_correct = 0
        num_total = 0
        num_tp = 0
        num_tn = 0
        num_fp = 0
        num_fn = 0

        val_loss += torch.sum(torch.stack(self.validation_step_outputs))
        self.validation_step_outputs.clear()

        self.log("loss/val", val_loss)

    def configure_optimizers(self):
        return torch.optim.Adam(self.parameters(), lr = 3e-4)