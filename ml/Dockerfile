
FROM python:3.10

RUN mkdir /app && \
    pip install torch==2.0 --index-url https://download.pytorch.org/whl/cpu && \
    pip install torch_geometric && \
    pip install zipfile-zstd && \
    pip install pyg_lib torch_scatter torch_sparse torch_cluster torch_spline_conv -f https://data.pyg.org/whl/torch-2.0.0+cpu.html 

COPY evaluate.py identify_x86_graph.py /app/

WORKDIR /app
ENTRYPOINT ["python", "evaluate.py"]