rust-cpython-json [![Build Status](https://travis-ci.org/ianweller/rust-cpython-json.svg?branch=master)](https://travis-ci.org/ianweller/rust-cpython-json)
=================

cpython-json converts native Python objects (via [cpython](https://crates.io/crates/cpython) `PyObject`s) to [serde_json](https://crates.io/crates/serde_json) `Value`s and back again.  

It was developed for [crowbar](https://crates.io/crates/crowbar), a shim for writing native Rust code in [AWS Lambda](https://aws.amazon.com/lambda/) using the Python execution environment. Because Lambda is a JSON-in, JSON-out API, all objects passing through crowbar are JSON serializable.

Values are not actually converted to JSON as part of this process; serializing and deserializing JSON is slow. Instead, `PyObject`s are natively casted to a reasonably matching type of `Value`, and `PyObject`s are created directly from pattern-matching `Value`s.

Data types that the Python `json` module can convert to JSON can be converted with this. (If you find something that works in the Python `json` module that doesn't work in `cpython-json`, please [file an issue](https://github.com/ianweller/rust-cpython-json/issues) with your test case.)

## Usage

Add `cpython-json` to your `Cargo.toml` alongside `cpython`:

```toml
[dependencies]
cpython = "*"
cpython-json = "0.1"
```

Similar to `cpython`, Python 3 is used by default. To use Python 2:

```toml
[dependencies.cpython-json]
version = "0.1"
default-features = false
features = ["python27-sys"]
```

Example code which reads `sys.hexversion` and bitbangs something resembling the version string
(release level and serial not included for brevity):

```rust
extern crate cpython;
extern crate cpython_json;
extern crate serde_json;

use cpython::*;
use cpython_json::to_json;
use serde_json::Value;

fn main() {
    let gil = Python::acquire_gil();
    println!("{}", version(gil.python()).expect("failed to get Python version"));
}

fn version(py: Python) -> PyResult<String> {
    let sys = py.import("sys")?;
    let py_hexversion = sys.get(py, "hexversion")?;
    let hexversion = match to_json(py, &py_hexversion).map_err(|e| e.to_pyerr(py))? {
        Value::U64(x) => x,
        Value::I64(x) => x as u64,
        _ => panic!("hexversion is not an int"),
    };

    Ok(format!("{}.{}.{}", hexversion >> 24, hexversion >> 16 & 0xff, hexversion >> 8 & 0xff))
}
```
