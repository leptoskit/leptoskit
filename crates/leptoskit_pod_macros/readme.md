# leptoskit_pod_macros

`#[pod::data]` derives `PodEncode` / `PodDecode` for the closed LK Pod type
grammar. Field ordinals and enum discriminators are assigned in source order
and frozen by migration history — authors never write field ids.
