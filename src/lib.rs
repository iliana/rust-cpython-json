//! cpython-json converts native Python objects (via cpython `PyObject`s) to `serde_json::Value`s
//! and back again.
//!
//! It was developed for [crowbar](https://crates.io/crates/crowbar), a shim for writing native
//! Rust code in [AWS Lambda](https://aws.amazon.com/lambda/) using the Python execution
//! environment. Because Lambda is a JSON-in, JSON-out API, all objects passing through crowbar are
//! JSON serializable.
//!
//! Values are not actually converted to JSON as part of this process; serializing and
//! deserializing JSON is slow. Instead, `PyObject`s are natively casted to a reasonably matching
//! type of `Value`, and `PyObject`s are created directly from pattern-matching `Value`s.
//!
//! Data types that the Python `json` module can convert to JSON can be converted with this. (If
//! you find something that works in the Python `json` module that doesn't work in `cpython-json`,
//! please [file an issue](https://github.com/ilianaw/rust-cpython-json/issues) with your test
//! case.)
//!
//! ## Usage
//!
//! Add `cpython-json` to your `Cargo.toml` alongside `cpython`:
//!
//! ```toml
//! [dependencies]
//! cpython = "*"
//! cpython-json = "0.3"
//! ```
//!
//! Similar to `cpython`, Python 3 is used by default. To use Python 2:
//!
//! ```toml
//! [dependencies.cpython-json]
//! version = "0.3"
//! default-features = false
//! features = ["python27-sys"]
//! ```
//!
//! Example code which reads `sys.hexversion` and bitbangs something resembling the version string
//! (release level and serial not included for brevity):
//!
//! ```rust
//! extern crate cpython;
//! extern crate cpython_json;
//! extern crate serde_json;
//!
//! use cpython::*;
//! use cpython_json::to_json;
//! use serde_json::Value;
//!
//! fn main() {
//!     let gil = Python::acquire_gil();
//!     println!("{}", version(gil.python()).expect("failed to get Python version"));
//! }
//!
//! fn version(py: Python) -> PyResult<String> {
//!     let sys = py.import("sys")?;
//!     let py_hexversion = sys.get(py, "hexversion")?;
//!     let hexversion = match to_json(py, &py_hexversion).map_err(|e| e.to_pyerr(py))? {
//!         Value::Number(x) => x.as_i64().expect("hexversion is not an int"),
//!         _ => panic!("hexversion is not an int"),
//!     };
//!
//!     Ok(format!("{}.{}.{}", hexversion >> 24, hexversion >> 16 & 0xff, hexversion >> 8 & 0xff))
//! }
//! ```

extern crate cpython;
#[macro_use]
extern crate quick_error;
extern crate serde_json;

use cpython::*;
use serde_json::value::Value;
use std::convert::From;

quick_error! {
    /// The `Error` enum returned by this crate.
    ///
    /// Most of the time you will just want a `PyErr`, and `to_pyerr` will convert to one for you.
    #[derive(Debug)]
    pub enum JsonError {
        /// A Python exception occurred.
        PythonError(err: PyErr) {
            from()
        }
        /// The PyObject passed could not be converted to a `serde_json::Value` because of a
        /// `serde_json` error.
        SerdeJsonError(err: serde_json::Error) {
            from()
        }
        /// The PyObject passed could not be converted to a `serde_json::Value` object because we
        /// didn't recognize it as a valid JSON-supported type.
        ///
        /// The error tuple is the type name, then the `repr` of the object.
        ///
        /// This usually means that Python's `json` module wouldn't be able to serialize the object
        /// either. If the Python `json` module works but `cpython-json` doesn't, please [file an
        /// issue] (https://github.com/ilianaw/rust-cpython-json/issues) with your test case.
        TypeError(type_name: String, repr: PyResult<String>) {}
        /// A `dict` key was not a string object, and so it couldn't be converted to an object. JSON
        /// object keys must always be strings.
        DictKeyNotString(obj: PyObject) {}
        /// A number provided is not a valid JSON float (infinite and NaN values are not supported
        /// in JSON).
        InvalidFloat {}
        /// The serde_json crate lied to us and a `Number` is neither u64, i64, or f64.
        ImpossibleNumber {}
    }
}

impl JsonError {
    /// Convenience method for converting a `JsonError` to a `PyErr`.
    pub fn to_pyerr(&self, py: Python) -> PyErr {
        match *self {
            JsonError::PythonError(ref err) => err.clone_ref(py),
            JsonError::SerdeJsonError(_) => {
                PyErr {
                    ptype: cpython::exc::RuntimeError::type_object(py).into_object(),
                    pvalue: Some(PyString::new(py, "serde_json error").into_object()),
                    ptraceback: None,
                }
            }
            JsonError::TypeError(_, ref repr) => {
                match *repr {
                    Ok(ref repr) => {
                        PyErr {
                            ptype: cpython::exc::TypeError::type_object(py).into_object(),
                            pvalue: Some(PyUnicode::new(py,
                                                        &format!("{} is not JSON serializable",
                                                                repr))
                                                 .into_object()),
                            ptraceback: None,
                        }
                    }
                    Err(ref err) => err.clone_ref(py),
                }
            }
            JsonError::DictKeyNotString(_) => {
                PyErr {
                    ptype: cpython::exc::TypeError::type_object(py).into_object(),
                    pvalue: Some(PyString::new(py, "keys must be a string").into_object()),
                    ptraceback: None,
                }
            }
            JsonError::InvalidFloat => {
                PyErr {
                    ptype: cpython::exc::ValueError::type_object(py).into_object(),
                    pvalue: Some(PyString::new(py, "inf and nan are not supported in JSON").into_object()),
                    ptraceback: None,
                }
            }
            JsonError::ImpossibleNumber => {
                PyErr {
                    ptype: cpython::exc::ValueError::type_object(py).into_object(),
                    pvalue: Some(PyString::new(py, "a value was somehow not an integer or float").into_object()),
                    ptraceback: None,
                }
            }
        }
    }
}

/// Convert from a `cpython::PyObject` to a `serde_json::Value`.
pub fn to_json(py: Python, obj: &PyObject) -> Result<Value, JsonError> {
    macro_rules! cast {
        ($t:ty, $f:expr) => {
            if let Ok(val) = obj.cast_as::<$t>(py) {
                return $f(val);
            }
        }
    }

    macro_rules! extract {
        ($t:ty) => {
            if let Ok(val) = obj.extract::<$t>(py) {
                return serde_json::value::to_value(val).map_err(JsonError::SerdeJsonError);
            }
        }
    }

    cast!(PyDict, |x: &PyDict| {
        let mut map = serde_json::Map::new();
        for (key_obj, value) in x.items(py) {
            let key = if key_obj == py.None() {
                Ok("null".to_string())
            } else if let Ok(val) = key_obj.extract::<bool>(py) {
                Ok(if val {
                       "true".to_string()
                   } else {
                       "false".to_string()
                   })
            } else if let Ok(val) = key_obj.str(py) {
                Ok(val.to_string(py)?.into_owned())
            } else {
                Err(JsonError::DictKeyNotString(key_obj))
            };
            map.insert(key?, to_json(py, &value)?);
        }
        Ok(Value::Object(map))
    });

    cast!(PyList,
          |x: &PyList| Ok(Value::Array(try!(x.iter(py).map(|x| to_json(py, &x)).collect()))));
    cast!(PyTuple,
          |x: &PyTuple| Ok(Value::Array(try!(x.iter(py).map(|x| to_json(py, x)).collect()))));

    extract!(String);
    extract!(bool);

    cast!(PyFloat,
          |x: &PyFloat| match serde_json::Number::from_f64(x.value(py)) {
              Some(n) => Ok(Value::Number(n)),
              None => Err(JsonError::InvalidFloat),
          });

    extract!(u64);
    extract!(i64);

    if obj == &py.None() {
        return Ok(Value::Null);
    }

    // At this point we can't cast it, set up the error object
    let repr = obj.repr(py)
        .and_then(|x| x.to_string(py).and_then(|y| Ok(y.into_owned())));
    Err(JsonError::TypeError(obj.get_type(py).name(py).into_owned(), repr))
}

/// Convert from a `serde_json::Value` to a `cpython::PyObject`.
pub fn from_json(py: Python, json: Value) -> Result<PyObject, JsonError> {
    macro_rules! obj {
        ($x:ident) => {
            Ok($x.into_py_object(py).into_object())
        }
    }

    match json {
        Value::Number(x) => {
            if let Some(n) = x.as_u64() {
                obj!(n)
            } else if let Some(n) = x.as_i64() {
                obj!(n)
            } else if let Some(n) = x.as_f64() {
                obj!(n)
            } else {
                // We should never get to this point
                Err(JsonError::ImpossibleNumber)
            }
        }
        Value::String(x) => Ok(PyUnicode::new(py, &x).into_object()),
        Value::Bool(x) => obj!(x),
        Value::Array(vec) => {
            let mut elements = Vec::new();
            for item in vec {
                elements.push(from_json(py, item)?);
            }
            Ok(PyList::new(py, &elements[..]).into_object())
        }
        Value::Object(map) => {
            let dict = PyDict::new(py);
            for (key, value) in map {
                dict.set_item(py, key, from_json(py, value)?)?;
            }
            Ok(dict.into_object())
        }
        Value::Null => Ok(py.None()),
    }
}

#[cfg(test)]
mod tests {
    use cpython::*;
    use cpython::exc::TypeError;
    use serde_json;
    use std::fs::File;
    use std::io::{BufRead, BufReader};
    use super::*;

    #[test]
    fn test_json() {
        let gil = Python::acquire_gil();
        let py = gil.python();

        // use operator.__eq__ to determine equality of PyObjects
        let operator = py.import("operator").unwrap();

        for line in BufReader::new(&File::open("testdata/to_json.txt").unwrap()).lines() {
            let line = line.unwrap();
            if line == "" || line.starts_with("#") {
                continue;
            }
            let mut line: Vec<_> = line.split("\t").collect();

            if line.len() == 2 {
                line.push("");
            }
            assert_eq!(line.len(), 3);

            // test to_json
            let json = serde_json::from_str(line[1]).unwrap();
            if !line[2].contains("skip_to") {
                let obj = py.eval(line[0], None, None).unwrap();
                assert_eq!(to_json(py, &obj).unwrap(),
                           json,
                           "to_json: {} != {}",
                           line[0],
                           line[1]);
            }

            // test from_json
            if !line[2].contains("skip_from") {
                let obj = py.eval(line[0], None, None).unwrap();
                let eq = operator
                    .call(py,
                          "__eq__",
                          PyTuple::new(py, &[from_json(py, json).unwrap(), obj]),
                          None)
                    .unwrap();
                assert!(eq.extract::<bool>(py).unwrap(),
                        "from_json: {} != {}",
                        line[0],
                        line[1]);
            }
        }
    }

    #[test]
    fn test_unserializable() {
        let gil = Python::acquire_gil();
        let py = gil.python();

        // datetime.datetime objects are not JSON serializable
        let datetime = py.import("datetime").unwrap();
        let min = datetime
            .get(py, "datetime")
            .unwrap()
            .getattr(py, "min")
            .unwrap();
        let err = to_json(py, &min).unwrap_err().to_pyerr(py);
        assert_eq!(err.ptype, TypeError::type_object(py).into_object());
        assert_eq!(err.pvalue.unwrap().to_string(),
                   "datetime.datetime(1, 1, 1, 0, 0) is not JSON serializable");
        assert_eq!(err.ptraceback, None);
    }

    #[test]
    /// The compiler already makes sure that JsonError can derive Debug, but kcov doesn't know
    /// that. This makes JsonError's #[derive(Debug)] show as covered code.
    fn test_jsonerror_debug() {
        let gil = Python::acquire_gil();
        let py = gil.python();
        println!("{:?}", JsonError::DictKeyNotString(py.None()));
    }

    #[test]
    fn test_to_pyerr_python() {
        let gil = Python::acquire_gil();
        let py = gil.python();

        let err = JsonError::PythonError(JsonError::DictKeyNotString(py.None()).to_pyerr(py))
            .to_pyerr(py);
        assert_eq!(err.ptype, TypeError::type_object(py).into_object());
        assert_eq!(err.pvalue.unwrap().to_string(), "keys must be a string");
        assert_eq!(err.ptraceback, None);
    }

    #[test]
    fn test_to_pyerr_type_failed_repr() {
        let gil = Python::acquire_gil();
        let py = gil.python();

        let err = JsonError::TypeError("datetime.datetime".to_string(),
                                       Err(JsonError::DictKeyNotString(py.None()).to_pyerr(py)))
                .to_pyerr(py);
        assert_eq!(err.ptype, TypeError::type_object(py).into_object());
        assert_eq!(err.pvalue.unwrap().to_string(), "keys must be a string");
        assert_eq!(err.ptraceback, None);
    }

    #[test]
    fn test_to_pyerr_dkns() {
        let gil = Python::acquire_gil();
        let py = gil.python();

        let err = JsonError::DictKeyNotString(py.None()).to_pyerr(py);
        assert_eq!(err.ptype, TypeError::type_object(py).into_object());
        assert_eq!(err.pvalue.unwrap().to_string(), "keys must be a string");
        assert_eq!(err.ptraceback, None);
    }
}
