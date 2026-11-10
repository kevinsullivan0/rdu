"""Strict full-output adapters; they retain names only in memory for validation."""
import re
from pathlib import PurePosixPath


def lines(output,max_bytes=65536):
    if len(output)>max_bytes or not output.endswith(b'\n') or b'\r' in output or b'\x1b' in output:
        raise ValueError('Output bound, line ending or terminal sequence coverage')
    try:return output.decode('utf8',errors='strict').splitlines()
    except UnicodeError as error:raise ValueError('Lossless UTF8 output coverage') from error


def insert(rows,path,size):
    if path in rows:raise ValueError('Duplicate retained output node')
    rows[path]=size


def parse_rdu_tree(output,max_bytes=65536):
    text=lines(output,max_bytes);rows={};markers={};order={}
    if not text:raise ValueError('Missing TOTAL')
    footer=re.fullmatch(r' *([0-9]+)  TOTAL \(([0-9]+) entries, ([0-9]+) directories, ([0-9]+) errors; ([0-9]+\.[0-9]{3})s\)',text[-1])
    if not footer:raise ValueError('Malformed RDU TOTAL footer')
    insert(rows,'',int(footer[1]))
    for line in text[:-1]:
        match=re.fullmatch(r' *([0-9]+)  ([^\r\n\x1b]+)',line)
        if not match:raise ValueError('Malformed RDU full-output row')
        display=match[2];directory=display.endswith('/');path=display[:-1] if directory else display
        parts=PurePosixPath(path).parts
        if not parts or path.startswith('/') or any(p in ('..','.') for p in parts) or str(PurePosixPath(path))!=path:
            raise ValueError('Noncanonical relative output path')
        insert(rows,path,int(match[1]));markers[path]=directory
        parent=path.rpartition('/')[0];order.setdefault(parent,[]).append(path)
    return {'rows':rows,'directory_markers':markers,'sibling_order':order,
            'counts':{'files':int(footer[2]),'directories':int(footer[3]),'errors':int(footer[4])}}


def parse_dua_tree(output,root,sentinel,max_bytes=65536):
    rows={};order={};stack=[];roots=[];sentinel_seen=False
    for line in lines(output,max_bytes):
        match=re.fullmatch(r' *([0-9]+) b ([^\r\n\x1b]+)',line)
        if not match:raise ValueError('Malformed dua bytes/tree row')
        size=int(match[1]);display=match[2];spaces=len(display)-len(display.lstrip(' '))
        if spaces%2:raise ValueError('Odd tree indentation')
        depth=spaces//2;name=display[spaces:]
        if depth==0:
            roots.append(size);stack=[]
            if name==sentinel:
                if sentinel_seen or size!=0:raise ValueError('Sentinel duplicate/nonzero')
                sentinel_seen=True;stack=[None];continue
            if name!=root:raise ValueError('Unexpected root row')
            path=''
        else:
            if len(stack)<depth or stack[depth-1] is None or '/' in name or name in ('.','..',''):
                raise ValueError('Missing parent, sentinel descendants or invalid basename')
            parent=stack[depth-1];path=parent+'/'+name if parent else name
            order.setdefault(parent,[]).append(path)
        insert(rows,path,size);stack=stack[:depth];stack.append(path)
    if not sentinel_seen or '' not in rows or len(roots)!=2 or roots!=sorted(roots):
        raise ValueError('Root/sentinel or root sort contract')
    return {'rows':rows,'sibling_order':order,'counts':None}


def validate_rows(parsed,model,metric,tool):
    rows=parsed['rows'];nodes=model['nodes']
    if set(rows)!=set(nodes):raise ValueError('Full retained node set mismatch')
    if rows['']!=model['totals'][metric]:raise ValueError('Root bytes mismatch')
    if parsed['counts'] is not None:
        for key in ('files','directories','errors'):
            if parsed['counts'][key]!=model['totals'][key]:raise ValueError('Exposed footer count mismatch')
    groups={};children={name:[] for name,n in nodes.items() if n['kind']=='directory'}
    for name,node in nodes.items():
        if name:
            parent=name.rpartition('/')[0]
            if parent not in children:raise ValueError('Output edge has nondirectory parent')
            children[parent].append(name)
            if tool=='rdu' and parsed['directory_markers'][name]!=(node['kind']=='directory'):
                raise ValueError('Directory marker mismatch')
        if node['kind']=='directory':continue
        key=node.get('hardlink')
        if key is not None:
            if rows[name] not in (0,node[metric]):raise ValueError('Hardlink charge value mismatch')
            groups.setdefault(tuple(key),[]).append(name)
        elif rows[name]!=node[metric]:raise ValueError('Nondirectory own bytes mismatch')
    for names in groups.values():
        if sum(rows[name] for name in names)!=nodes[names[0]][metric]:raise ValueError('Hardlink charged more/less than once')
    for parent,names in children.items():
        if rows[parent]!=nodes[parent][metric]+sum(rows[name] for name in names):
            raise ValueError('Directory own plus immediate children subtotal mismatch')
        sequence=parsed['sibling_order'].get(parent,[])
        if set(sequence)!=set(names) or len(sequence)!=len(names):raise ValueError('Child edge set mismatch')
        for a,b in zip(sequence,sequence[1:]):
            if rows[a]>rows[b]:raise ValueError('Size ascending sibling order violated')
            if tool=='rdu' and rows[a]==rows[b] and a.rsplit('/',1)[-1].encode()<b.rsplit('/',1)[-1].encode():
                raise ValueError('RDU reverse name tie order violated')
    return {'complete_nodes_and_edges':True,'nodes':len(nodes),'edges':len(nodes)-1,
            'directory_subtotals':True,'hardlink_charge_sums':True,'hardlink_groups':len(groups),
            'ascending_size_order':True,'exact_root_bytes':rows[''],'exposed_counts_validated':parsed['counts'] is not None}
