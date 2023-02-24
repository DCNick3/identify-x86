
import re
import csv

REX = re.compile(r"^.*identify_x86_datatool: ([^:]+): ([0-9]+) nodes in ([0-9.]+)s.*$")

out = csv.writer(open('big-run.csv', 'w'))
out.writerow(['sample_name', 'size', 'time'])

for line in open('big-run.log'):
    m = REX.match(line)
    if m:
        sample_name = m.group(1)
        size = m.group(2)
        time = m.group(3)
        out.writerow([sample_name, size, time])
        
