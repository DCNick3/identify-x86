
import re
import csv

REX = re.compile(r"^.*identify_x86_datatool:\s+([^:]+): ([0-9]+) nodes ([0-9]+) edges in ([0-9.]+)s.*$")

out = csv.writer(open('big-run-2.csv', 'w'))
out.writerow(['sample_name', 'nodes', 'edges', 'time'])

for line in open('big-run-2.log'):
    m = REX.match(line)
    if m:
        sample_name = m.group(1)
        nodes = m.group(2)
        edges = m.group(3)
        time = m.group(4)
        out.writerow([sample_name, nodes, edges, time])
        
