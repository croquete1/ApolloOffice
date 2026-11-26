"""Read OpenCode's actual verbose metadata; never infer pricing from model names."""
import json,pathlib,subprocess,time,os,tempfile
from opencode_bridge import CATALOG,allowed_models,catalog_json_options
from launch import STATE

def parse_models(text):
 decoder=json.JSONDecoder(**catalog_json_options());rows=[];rest=text
 while rest.strip():
  # `models --verbose` prints an ID line followed by its complete JSON object.
  header,separator,body=rest.lstrip().partition('\n')
  body=body.lstrip()
  if not separator or not body.startswith('{'):
   raise ValueError('Incomplete model metadata record')
  try:value,used=decoder.raw_decode(body)
  except RecursionError as exc:raise ValueError('Free-model catalog nesting too deep') from exc
  rest=body[used:]
  if rest.split('\n',1)[0].strip():
   raise ValueError('Unexpected text after model metadata')
  if not isinstance(value,dict):raise ValueError('Expected model object')
  ident=value.get('id');provider=value.get('providerID')
  if not isinstance(ident,str) or not ident.strip() or not isinstance(provider,str) or not provider.strip():
   raise ValueError('Invalid model identity in returned metadata')
  full_id=provider+'/'+ident
  if header.strip()!=full_id:
   raise ValueError('Model metadata identity does not match CLI header')
  value['full_id']=full_id;rows.append(value)
 if not rows:raise ValueError('No model metadata returned')
 return rows

def main():
 launcher=str(pathlib.Path(__file__).with_name('launch.py'))
 output=subprocess.run([launcher,'models','opencode','--verbose','--refresh'],check=True,capture_output=True,text=True,timeout=90)
 rows=parse_models(output.stdout);CATALOG.parent.mkdir(parents=True,exist_ok=True,mode=0o700)
 fd,name=tempfile.mkstemp(prefix='.catalog-',dir=CATALOG.parent)
 try:
  with os.fdopen(fd,'w') as f:json.dump({'updated_at':time.time(),'models':rows},f);f.flush();os.fsync(f.fileno())
  free=allowed_models(pathlib.Path(name))
  if not free:raise ValueError('No active zero-price models in returned metadata')
  os.replace(name,CATALOG)
  print(json.dumps({'free_models':sorted(free),'catalog':str(CATALOG)}))
 finally:
  pathlib.Path(name).unlink(missing_ok=True)
if __name__=='__main__':main()
