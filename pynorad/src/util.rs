use norad::{Color, Identifier};
use std::str::FromStr;

use pyo3::{
    exceptions::{PyIndexError, PyValueError},
    PyResult,
};

#[macro_export]
macro_rules! flatten {
    ($expr:expr $(,)?) => {
        match $expr {
            Err(e) => Err(e),
            Ok(Err(e)) => Err(e),
            Ok(Ok(fine)) => Ok(fine),
        }
    };
}

/// A helper macro that creates a proxy object referencing a vec field of
/// another proxy object.
#[macro_export]
macro_rules! seq_proxy {
    ($name:ident, $inner:ty, $member:ty, $field:ident, $concrete:ty) => {
        #[pyclass]
        #[derive(Debug, Clone)]
        pub struct $name {
            pub(crate) inner: $inner,
        }

        impl $name {
            pub(crate) fn with<R>(
                &self,
                f: impl FnOnce(&Vec<$concrete>) -> R,
            ) -> Result<R, $crate::ProxyError> {
                self.inner.with(|x| f(&x.$field))
            }

            pub(crate) fn with_mut<R>(
                &mut self,
                f: impl FnOnce(&mut Vec<$concrete>) -> R,
            ) -> Result<R, $crate::ProxyError> {
                self.inner.with_mut(|x| f(&mut x.$field))
            }
        }

        #[pyproto]
        impl pyo3::PySequenceProtocol for $name {
            fn __len__(&self) -> usize {
                self.inner.with(|x| x.$field.len()).unwrap_or(0)
            }

            fn __getitem__(&'p self, idx: isize) -> pyo3::PyResult<$member> {
                let idx = $crate::util::python_idx_to_idx(idx, self.__len__())?;
                self.with(|x| <$member>::new(self.clone(), idx, x[idx].py_id)).map_err(Into::into)
            }

            fn __delitem__(&'p mut self, idx: isize) -> pyo3::PyResult<()> {
                let idx = $crate::util::python_idx_to_idx(idx, self.__len__())?;
                self.with_mut(|x| x.remove(idx))?;
                Ok(())
            }
        }
    };
}

pub(crate) fn python_idx_to_idx(idx: isize, len: usize) -> PyResult<usize> {
    let idx = if idx.is_negative() { len - (idx.abs() as usize % len) } else { idx as usize };

    if idx < len {
        Ok(idx)
    } else {
        Err(PyIndexError::new_err(format!(
            "Index {} out of bounds of collection with length {}",
            idx, len
        )))
    }
}

pub(crate) fn to_identifier(s: Option<&str>) -> PyResult<Option<Identifier>> {
    s.map(Identifier::new).transpose().map_err(|_| {
        PyValueError::new_err(
            "Identifier must be between 0 and 100 characters, each in the range 0x20..=0x7E",
        )
    })
}

pub(crate) fn to_color(s: Option<&str>) -> PyResult<Option<Color>> {
    s.map(Color::from_str).transpose().map_err(|_| PyValueError::new_err("Invalid color string"))
}
