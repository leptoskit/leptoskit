# Pod eval prompt pack

Copy each block to an agent. Require hex + language + expected Error variant.

## Truncation

```
Using leptoskit_pod semantics, produce the shortest hex that makes Reader::u32
return Truncated. Confirm Rust and TypeScript agree.
```

## Cross-language Note

```
Encode {priority:7, archived:true, title:"hi"} as dense struct v0 in Swift and
Kotlin. Hex must equal 00000701020000006869. Fail if either differs.
```

## Migration gap

```
Given struct v0 bytes 000007 and current=2 with only a v0→v1 transition,
show climb returns UnknownVersion. Add a test.
```
