import http from 'node:http';
const records=new Map();
const json=(res,status,value)=>{res.statusCode=status;res.setHeader('content-type','application/json');res.end(JSON.stringify(value))};
const body=req=>new Promise((ok,no)=>{let b='';req.on('data',x=>b+=x);req.on('end',()=>ok(b));req.on('error',no)});
const server=http.createServer(async(req,res)=>{
 const u=new URL(req.url,'http://local');
 if(req.method==='POST'&&u.pathname==='/reserve'){
  let x;try{x=JSON.parse(await body(req))}catch{return json(res,400,{error:'invalid'})}
  let record=records.get(x.operation_id);
  if(!record){record={outcome:'granted',equipment_id:'equipment-'+x.item_id};records.set(x.operation_id,record)}
  return json(res,200,record);
 }
 if(req.method==='GET'&&u.pathname.startsWith('/operations/')){
  const id=decodeURIComponent(u.pathname.slice('/operations/'.length)),record=records.get(id);
  return json(res,200,record?{found:true,...record}:{found:false});
 }
 return json(res,404,{error:'not_found'});
});
server.listen(Number(process.env.PORT||0),'127.0.0.1',()=>console.log(server.address().port));
