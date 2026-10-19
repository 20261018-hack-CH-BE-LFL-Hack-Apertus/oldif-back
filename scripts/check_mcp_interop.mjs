#!/usr/bin/env node
// Explicit SDK installation path keeps this optional client independent of other repositories.
import fs from 'node:fs';
import path from 'node:path';
import {pathToFileURL} from 'node:url';
const sdk=process.env.OLDIF_MCP_SDK_DIR;
const base=process.env.OLDIF_BACKEND_URL;
const origin=process.env.OLDIF_APP_ORIGIN;
if(!sdk||!base||!origin)throw new Error('Set OLDIF_MCP_SDK_DIR, OLDIF_BACKEND_URL and OLDIF_APP_ORIGIN');
const {Client}=await import(pathToFileURL(path.join(sdk,'dist/esm/client/index.js')));
const {StreamableHTTPClientTransport}=await import(pathToFileURL(path.join(sdk,'dist/esm/client/streamableHttp.js')));
const response=await fetch(base+'/api/auth/demo',{method:'POST',headers:{Origin:origin,'Content-Type':'application/json'},body:'{}'});
if(!response.ok)throw new Error('Demo authentication failed');
const cookie=response.headers.get('set-cookie').split(';')[0];
const checks=[],errors=[];
const client=new Client({name:'oldif-independent-validation',version:'1.0.0'},{capabilities:{}});
client.onerror=error=>errors.push({name:error.name,message:error.message});
const transport=new StreamableHTTPClientTransport(new URL(base+'/api/mcp'),{requestInit:{headers:{Cookie:cookie,Origin:origin}},reconnectionOptions:{maxReconnectionDelay:1000,initialReconnectionDelay:100,reconnectionDelayGrowFactor:1,maxRetries:0}});
const check=(name,passed)=>checks.push({check:name,passed:!!passed});
try{
  await client.connect(transport,{timeout:10000});check('official_sdk_initialize',true);
  const tools=await client.listTools();check('only_bounded_read_tools',tools.tools.length===2&&tools.tools.every(t=>['search_archives','get_source'].includes(t.name)));
  const search=await client.callTool({name:'search_archives',arguments:{query:'Alpthal'}});const results=JSON.parse(search.content[0].text);check('real_search_source_ids',results.length>0&&results.length<=6&&results.every(r=>r.id&&r.sha256));
  const source=await client.callTool({name:'get_source',arguments:{id:'stapfer-2027'}});const record=JSON.parse(source.content[0].text);check('real_source_provenance',record.id==='stapfer-2027'&&record.text.length>0&&record.sha256.length===64);
  let forbidden=false;try{await client.callTool({name:'run_shell',arguments:{command:'echo forbidden'}});}catch{forbidden=true;}check('unknown_tool_rejected',forbidden);
  let extra=false;try{const result=await client.callTool({name:'get_source',arguments:{id:'stapfer-2027',url:'https://invalid.example'}});extra=result.isError===true;}catch{extra=true;}check('extra_arguments_rejected',extra);
}catch(error){errors.push({name:error.name,message:error.message});check('protocol_execution',false);}
finally{await client.close();}
const receipt={validation_class:'official TypeScript MCP SDK over real authenticated HTTP',sdk_version:JSON.parse(fs.readFileSync(path.join(sdk,'package.json'))).version,checks,passed:checks.filter(c=>c.passed).length,failed:checks.filter(c=>!c.passed).length,protocol_errors:errors,provider_calls:0};
if(process.env.OLDIF_MCP_RECEIPT)fs.writeFileSync(process.env.OLDIF_MCP_RECEIPT,JSON.stringify(receipt,null,2)+'\n');
console.log(JSON.stringify({passed:receipt.passed,failed:receipt.failed,protocol_errors:errors}));
process.exitCode=receipt.failed||errors.length?1:0;
