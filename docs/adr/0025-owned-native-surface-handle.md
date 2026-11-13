---
status: accepted
---

# Owned native surface handle with one unsafe constructor in oxiview-base

Place the owned `NativeSurfaceHandle` and non-exhaustive `NativeSurfaceKind` in `oxiview-base`. M1 supports only a macOS `CAMetalLayer`. A private `NonZeroUsize` address makes the value `Send + Sync + Copy` without unsafe implementations. Safe accessors expose its kind and address, not a third-party type. Owned means the framework owns the representation; copying it does not retain the native object.

The producing platform adapter calls the sole unsafe constructor, `NativeSurfaceHandle::from_macos_metal_layer`, following the raw-window-handle model of declaring native validity obligations at production. The consuming rendering adapter reconstructs the native pointer and handles unsafe surface creation internally. Ports and `oxiview-app`, the composition root, remain free of unsafe. Platform and rendering adapters cannot depend on each other under [ADR-0021](0021-crate-layout-strategic-ddd.md).

The constructor requires a live, initialized layer with valid exposed pointer provenance and synchronized native mutations, usable for surface creation and presentation off the main thread. It remains attached and alive until window destruction has reached the window thread **and** every surface created from the handle has been released. Endpoint teardown signals that surface release is complete.

A Copy value cannot track when its last copy disappears. To allow safe renderer entry points without additional caller obligations, the constructor additionally requires the object to remain valid for surface creation and presentation for the process lifetime, even if a stale copy is consumed after destruction. Attachment is required only through the destruction-and-release condition above. This intentionally strong contract costs native resource retention; a future reclaimable design must change the handle ownership/lifetime model, not silently weaken this guarantee. S1 implements no retention or native backend.

## Considered Options

- **raw-window-handle in public APIs**: familiar interoperability, but leaks third-party types across contexts and ties ports to their evolution.
- **Plain-data handle plus unsafe create_surface called from oxiview-app**: moves lifetime proof to the composition root and introduces unsafe outside adapters without an enforceable production contract.
- **oxiview-render-wgpu depends on oxiview-platform**: couples two contexts and violates the registered dependency direction; the shared foundation is the appropriate common vocabulary.
- **An owning reference-counted native resource**: permits reclamation but cannot satisfy the requested Copy handle. Revisit ownership before relaxing the process-lifetime guarantee.

## Consequences

- `oxiview-base` is the one non-adapter crate allowed exactly this unsafe function declaration. Its item-level `#[allow(unsafe_code)]` is the smallest compiler-supported scope. No crate-level or module-level allowance is permitted.
- No unsafe block or unsafe impl is permitted anywhere in `crates/oxiview-base/src`; an integration test scans that tree and enforces the boundary, including the single allowance/declaration.
- The constructor contract carries all native validity obligations. Safe accessors alone never dereference native memory; rendering adapters remain responsible for respecting the documented native use and synchronization constraints.
- Other platforms add kinds only when their ports are implemented. The API remains unstable until independently implemented and tested, per the [API stability conventions](../conventions/api-stability.md).
