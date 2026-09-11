# Snake upstream A fixtures

Fresh source-derived inputs for snake semantic baseline `57170459b3d5ca175a1c57933058b569088bee0e`, Rust snake policy 13, and original policy 3. Reuses earlier fixture structure and harness entrypoints without relabeling historical observations. New captures are required.

`checks` exercises ordinary CHKDATA header/version behavior. `strings` exercises four language encodings and explicitly registered UTF-16 lone-surrogate replacement. Expected widths are candidates reused from the original provider and must be checked against the actual snake provider before acceptance.
