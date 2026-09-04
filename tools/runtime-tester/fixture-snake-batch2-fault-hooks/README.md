# Batch 2D final-fault hook oracle fixture

`enabled` covers successful BEFORE_THROW/BEFORE_ERROR dispatch and a secondary hook fault.
`disabled` maps the snake `DisableBeforeErrorThrow` configuration and proves neither hook runs.
Cases are observation inputs, not goldens; execute each case and compare the Rust capture with the isolated snake reference before advancing.
