#!/usr/bin/env python3
"""Execute this repository's trusted demo code cells without a Jupyter installation.

Does not execute arbitrary fetched notebooks. Outputs explicitly record this runner.
"""
import contextlib, io, json, pathlib
root=pathlib.Path(__file__).resolve().parents[1]
path=root/'notebooks/document-pipeline.ipynb'
notebook=json.loads(path.read_text())
namespace={'__name__':'__main__'}
for index,cell in enumerate(c for c in notebook['cells'] if c['cell_type']=='code'):
    output=io.StringIO()
    with contextlib.redirect_stdout(output): exec(compile(''.join(cell['source']),str(path),'exec'),namespace)
    cell['execution_count']=index+1
    cell['outputs']=[{'output_type':'stream','name':'stdout','text':output.getvalue().splitlines(keepends=True)}]
notebook['metadata']['validation']={'runner':'trusted Python code-cell executor','jupyter_kernel_executed':False,'provider_calls_enabled':False}
path.write_text(json.dumps(notebook,indent=2)+'\n')
print('Notebook code cells executed successfully; no Jupyter kernel claim.')
