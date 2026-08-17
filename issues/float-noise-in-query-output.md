# Query numbers can print 17 digits of float noise

Original problem (2026-08): mesh positions were f32, so `odm inspect`
bounds read `-25.600000023841858` where the model says `-25.6`. Fixed
2026-08-17 by the mesh f64 switch — bounds of authored geometry now print
clean.

What remains is ordinary f64 arithmetic noise: volume/area of CSG results
(and bounds of rotated/CSG'd geometry) are computed values and can still
print `1000.0000000000005`-style tails. Much rarer and always honest, but
the "where would rounding belong" question from the original issue still
applies if it bothers agents in practice: engine (clean JSON) vs CLI
(protocol keeps full precision). Revisit only on demonstrated annoyance.
