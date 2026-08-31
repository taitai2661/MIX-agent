import fs from 'node:fs';
import path from 'node:path';
import ts from '../apps/web/node_modules/typescript/lib/typescript.js';
const root=path.resolve('apps/web');
const config=ts.readConfigFile(path.join(root,'tsconfig.json'),ts.sys.readFile);
const parsed=ts.parseJsonConfigFileContent(config.config,ts.sys,root);
const host={...ts.sys,getScriptFileNames:()=>parsed.fileNames,getScriptVersion:()=> '0',getScriptSnapshot:f=>fs.existsSync(f)?ts.ScriptSnapshot.fromString(fs.readFileSync(f,'utf8')):undefined,getCurrentDirectory:()=>root,getCompilationSettings:()=>parsed.options,getDefaultLibFileName:o=>ts.getDefaultLibFilePath(o)};
const service=ts.createLanguageService(host);
for(const file of parsed.fileNames.filter(f=>f.endsWith('.tsx'))){
 for(const edit of service.organizeImports({type:'file',fileName:file},{},{})){
  let source=fs.readFileSync(edit.fileName,'utf8');
  for(const change of [...edit.textChanges].sort((a,b)=>b.span.start-a.span.start))source=source.slice(0,change.span.start)+change.newText+source.slice(change.span.start+change.span.length);
  fs.writeFileSync(edit.fileName,source);
 }
}
