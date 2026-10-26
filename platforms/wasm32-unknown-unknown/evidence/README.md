# wasm32 browser platform evidence

This platform declares MMIO behavior unmodeled. Its aperture exists to route
register accesses through the browser/CI peripheral host interface; it is not
evidence that a particular physical device model is correct. Deterministic
fixture devices are defined by the execution-test host harness.
