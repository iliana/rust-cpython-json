extern crate cpython;
extern crate serde_json;

use cpython::*;
use serde_json::value::Value;
use std::collections::BTreeMap;

#[derive(Debug)]
pub enum JsonError {
    PythonError(PyErr),
    TypeError(String, Option<String>),
    DictKeyNotString(PyObject),
}

macro_rules! pytry {
    ($x:expr) => {
        match $x {
            Ok(val) => val,
            Err(err) => return Err(JsonError::PythonError(err)),
        }
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
            if let Ok(val) = key_obj.cast_as::<PyString>(py) {
                map.insert(pytry!(val.to_string(py)).into_owned(),
                           try!(to_json(py, value)));
                continue;
            }
            return Err(JsonError::DictKeyNotString(key_obj));
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
    let repr = match obj.repr(py) {
        Ok(val) => {
            match val.to_string(py) {
                Ok(val) => Some(val.into_owned()),
                _ => None,
            }
        }
        _ => None,
    };
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
                elements.push(try!(from_json(py, item)));
            }
            Ok(PyList::new(py, &elements[..]).into_object())
        }
        Value::Object(map) => {
            let dict = PyDict::new(py);
            for (key, value) in map {
                pytry!(dict.set_item(py, key, try!(from_json(py, value))));
            }
            Ok(dict.into_object())
        }
        Value::Null => Ok(py.None()),
    }
}

#[cfg(test)]
mod tests {
    use cpython::*;
    use serde_json;
    use std::fs::File;
    use std::io::{BufRead, BufReader};
    use super::{to_json, from_json};

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
}
