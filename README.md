# oci-unpack

oci-unpack is a Rust crate to download and unpack [OCI images][images] from a
[container registry][registry].

The unpack process is running in a sandbox created with [Landlock], so it only
has access to the files beneath the target directory.

See the [API documentation][API] for more details.

[API]: https://docs.rs/oci-unpack
[Landlock]: https://landlock.io/
[images]: https://opencontainers.org/
[registry]: https://distribution.github.io/distribution/

## Example

The repository includes an CLI program in the [`examples` directory](./examples/).

```console
$ cargo run --quiet --release --example unpack -- alpine /tmp/alpine-unpack
```

## Alternatives

* [skopeo](https://github.com/containers/skopeo) can be used to download an OCI
  image from a registry.
* [umoci](https://github.com/opencontainers/umoci) can be used to unpack the layers.

The following commands are equivalent to the process implemented in this crate:

```console
$ skopeo copy docker://alpine oci:alpine-image:latest

$ umoci unpack --image alpine-image alpine-unpack
```
