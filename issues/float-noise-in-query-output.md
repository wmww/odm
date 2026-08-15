# Query numbers print 17 digits of f32 noise

`odm inspect` bounds read `-25.600000023841858` where the model says `-25.6`.
Mesh positions are stored as `f32`, so everything past ~7 significant digits
is conversion artefact — but bounds/volume/area come back as `f64` and print
in full. On a wide tree that noise is a real share of the output, and it
makes "is this at 25.6?" harder to answer than it should be.

Not obviously just a formatting fix: rounding to f32 precision is honest for
mesh-derived numbers, but volume/area are f64 Manifold results over f32
input, and the conformance suite compares numbers with explicit epsilons.
Decide where the rounding belongs (engine, so the JSON is clean, or CLI, so
the protocol keeps full precision) before doing it.
