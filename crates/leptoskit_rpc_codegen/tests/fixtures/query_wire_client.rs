use std::io::{Read, Write};
use std::net::TcpListener;
use std::collections::HashMap;
use serde_json::json;
#[tokio::main(flavor="current_thread")]
async fn main() {
 let listener=TcpListener::bind("127.0.0.1:0").unwrap();
 let address=listener.local_addr().unwrap();
 let observer=std::thread::spawn(move || {
   let mut requests=Vec::new();
   for _ in 0..3 {
     let (mut socket, _)=listener.accept().unwrap();
     socket.set_read_timeout(Some(std::time::Duration::from_secs(10))).unwrap();
     let mut bytes=Vec::new();
     let header_end=loop {
       let mut byte=[0]; socket.read_exact(&mut byte).unwrap(); bytes.push(byte[0]);
       if bytes.ends_with(b"\r\n\r\n") {break bytes.len();}
     };
     let headers=String::from_utf8(bytes).unwrap();
     let len=headers.lines().filter_map(|line|line.split_once(':')).find(|(key,_)|key.eq_ignore_ascii_case("content-length")).map_or(0,|(_,value)|value.trim().parse::<usize>().unwrap());
     let mut body=vec![0;len];socket.read_exact(&mut body).unwrap();
     let path=headers.lines().next().unwrap().split_whitespace().nth(1).unwrap();
     requests.push(json!({"query":path.split_once('?').map_or("",|(_,q)|q),"body":String::from_utf8(body).unwrap()}));
     socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\nContent-Type: application/json\r\n\r\n\"ok\"").unwrap();
     let _ = header_end;
   }
   requests
 });
 let client=wire::Client::with_server_url(format!("http://{address}"));
 for fallback in [None, Some(String::new())] {
   client.probe(Vec::new(),HashMap::new(),None,false,0,String::new(),fallback).await.unwrap();
 }
 client.probe(vec![None, Some("a +&é".into()), Some(String::new())], HashMap::from([("key +&é".into(), json!(null)), ("empty".into(), json!(""))]), None, false, 0, "a +&é".into(), None).await.unwrap();
 println!("{}",serde_json::to_string(&observer.join().unwrap()).unwrap());
}
