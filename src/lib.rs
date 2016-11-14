extern crate cpython;
extern crate serde_json;

use cpython::*;
use serde_json::value::Value;
use std::collections::BTreeMap;
use std::convert::From;

#[derive(Debug)]
pub enum JsonError {
    PythonError(PyErr),
    TypeError(String, PyResult<String>),
    DictKeyNotString(PyObject),
}

impl JsonError {
    pub fn to_pyerr(&self, py: Python) -> PyErr {
        match *self {
            JsonError::PythonError(ref err) => err.clone_ref(py),
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
        }
    }
}

impl From<PyErr> for JsonError {
    fn from(err: PyErr) -> JsonError {
        JsonError::PythonError(err)
    }
}

pub fn to_json(py: Python, obj: PyObject) -> Result<Value, JsonError> {
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
                return Ok(serde_json::value::to_value(val));
            }
        }
    }

    cast!(PyDict, |x: &PyDict| {
        let mut map = BTreeMap::new();
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
            map.insert(key?, to_json(py, value)?);
        }
        Ok(Value::Object(map))
    });

    macro_rules! to_vec {
        ($t:ty) => {
            |x: &$t| Ok(Value::Array(try!(x.iter(py).map(|x| to_json(py, x)).collect())));
        }
    }
    cast!(PyList, to_vec!(PyList));
    cast!(PyTuple, to_vec!(PyTuple));

    extract!(String);
    extract!(bool);

    cast!(PyFloat, |x: &PyFloat| Ok(Value::F64(x.value(py))));

    extract!(u64);
    extract!(i64);

    if obj == py.None() {
        return Ok(Value::Null);
    }

    // At this point we can't cast it, set up the error object
    let repr = obj.repr(py).and_then(|x| x.to_string(py).and_then(|y| Ok(y.into_owned())));
    Err(JsonError::TypeError(obj.get_type(py).name(py).into_owned(), repr))
}

pub fn from_json(py: Python, json: Value) -> Result<PyObject, JsonError> {
    macro_rules! obj {
        ($x:ident) => {
            Ok($x.into_py_object(py).into_object())
        }
    }

    match json {
        Value::I64(x) => obj!(x),
        Value::U64(x) => obj!(x),
        Value::F64(x) => obj!(x),
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
                assert_eq!(to_json(py, obj).unwrap(),
                           json,
                           "to_json: {} != {}",
                           line[0],
                           line[1]);
            }

            // test from_json
            if !line[2].contains("skip_from") {
                let obj = py.eval(line[0], None, None).unwrap();
                let eq = operator.call(py,
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
        let min = datetime.get(py, "datetime").unwrap().getattr(py, "min").unwrap();
        let err = to_json(py, min).unwrap_err().to_pyerr(py);
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
