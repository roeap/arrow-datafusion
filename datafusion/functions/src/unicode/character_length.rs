// Licensed to the Apache Software Foundation (ASF) under one
// or more contributor license agreements.  See the NOTICE file
// distributed with this work for additional information
// regarding copyright ownership.  The ASF licenses this file
// to you under the Apache License, Version 2.0 (the
// "License"); you may not use this file except in compliance
// with the License.  You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing,
// software distributed under the License is distributed on an
// "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied.  See the License for the
// specific language governing permissions and limitations
// under the License.

use crate::utils::{make_scalar_function, utf8_to_int_type};
use arrow::array::{
    Array, ArrayRef, ArrowPrimitiveType, AsArray, OffsetSizeTrait, PrimitiveArray,
    StringArrayType,
};
use arrow::datatypes::{ArrowNativeType, DataType, Int32Type, Int64Type};
use datafusion_common::Result;
use datafusion_expr::{
    ColumnarValue, Documentation, ScalarUDFImpl, Signature, Volatility,
};
use datafusion_macros::user_doc;
use std::any::Any;
use std::sync::Arc;

#[user_doc(
    doc_section(label = "String Functions"),
    description = "Returns the number of characters in a string.",
    syntax_example = "character_length(str)",
    sql_example = r#"```sql
> select character_length('Ångström');
+------------------------------------+
| character_length(Utf8("Ångström")) |
+------------------------------------+
| 8                                  |
+------------------------------------+
```"#,
    standard_argument(name = "str", prefix = "String"),
    related_udf(name = "bit_length"),
    related_udf(name = "octet_length")
)]
#[derive(Debug)]
pub struct CharacterLengthFunc {
    signature: Signature,
    aliases: Vec<String>,
}

impl Default for CharacterLengthFunc {
    fn default() -> Self {
        Self::new()
    }
}

impl CharacterLengthFunc {
    pub fn new() -> Self {
        use DataType::*;
        Self {
            signature: Signature::uniform(
                1,
                vec![Utf8, LargeUtf8, Utf8View],
                Volatility::Immutable,
            ),
            aliases: vec![String::from("length"), String::from("char_length")],
        }
    }
}

impl ScalarUDFImpl for CharacterLengthFunc {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn name(&self) -> &str {
        "character_length"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, arg_types: &[DataType]) -> Result<DataType> {
        utf8_to_int_type(&arg_types[0], "character_length")
    }

    fn invoke_with_args(
        &self,
        args: datafusion_expr::ScalarFunctionArgs,
    ) -> Result<ColumnarValue> {
        make_scalar_function(character_length, vec![])(&args.args)
    }

    fn aliases(&self) -> &[String] {
        &self.aliases
    }

    fn documentation(&self) -> Option<&Documentation> {
        self.doc()
    }
}

/// Returns number of characters in the string.
/// character_length('josé') = 4
/// The implementation counts UTF-8 code points to count the number of characters
fn character_length(args: &[ArrayRef]) -> Result<ArrayRef> {
    match args[0].data_type() {
        DataType::Utf8 => {
            let string_array = args[0].as_string::<i32>();
            character_length_general::<Int32Type, _>(string_array)
        }
        DataType::LargeUtf8 => {
            let string_array = args[0].as_string::<i64>();
            character_length_general::<Int64Type, _>(string_array)
        }
        DataType::Utf8View => {
            let string_array = args[0].as_string_view();
            character_length_general::<Int32Type, _>(string_array)
        }
        _ => unreachable!("CharacterLengthFunc"),
    }
}

fn character_length_general<'a, T, V>(array: V) -> Result<ArrayRef>
where
    T: ArrowPrimitiveType,
    T::Native: OffsetSizeTrait,
    V: StringArrayType<'a>,
{
    // String characters are variable length encoded in UTF-8, counting the
    // number of chars requires expensive decoding, however checking if the
    // string is ASCII only is relatively cheap.
    // If strings are ASCII only, count bytes instead.
    let is_array_ascii_only = array.is_ascii();
    let array = if array.null_count() == 0 {
        if is_array_ascii_only {
            let values: Vec<_> = (0..array.len())
                .map(|i| {
                    let value = array.value(i);
                    T::Native::usize_as(value.len())
                })
                .collect();
            PrimitiveArray::<T>::new(values.into(), None)
        } else {
            let values: Vec<_> = (0..array.len())
                .map(|i| {
                    let value = array.value(i);
                    if value.is_ascii() {
                        T::Native::usize_as(value.len())
                    } else {
                        T::Native::usize_as(value.chars().count())
                    }
                })
                .collect();
            PrimitiveArray::<T>::new(values.into(), None)
        }
    } else if is_array_ascii_only {
        let values: Vec<_> = (0..array.len())
            .map(|i| {
                if array.is_null(i) {
                    T::default_value()
                } else {
                    let value = array.value(i);
                    T::Native::usize_as(value.len())
                }
            })
            .collect();
        PrimitiveArray::<T>::new(values.into(), array.nulls().cloned())
    } else {
        let values: Vec<_> = (0..array.len())
            .map(|i| {
                if array.is_null(i) {
                    T::default_value()
                } else {
                    let value = array.value(i);
                    if value.is_ascii() {
                        T::Native::usize_as(value.len())
                    } else {
                        T::Native::usize_as(value.chars().count())
                    }
                }
            })
            .collect();
        PrimitiveArray::<T>::new(values.into(), array.nulls().cloned())
    };

    Ok(Arc::new(array))
}

#[cfg(test)]
mod tests {
    use crate::unicode::character_length::CharacterLengthFunc;
    use crate::utils::test::test_function;
    use arrow::array::{Array, Int32Array, Int64Array};
    use arrow::datatypes::DataType::{Int32, Int64};
    use datafusion_common::{Result, ScalarValue};
    use datafusion_expr::{ColumnarValue, ScalarUDFImpl};

    macro_rules! test_character_length {
        ($INPUT:expr, $EXPECTED:expr) => {
            test_function!(
                CharacterLengthFunc::new(),
                vec![ColumnarValue::Scalar(ScalarValue::Utf8($INPUT))],
                $EXPECTED,
                i32,
                Int32,
                Int32Array
            );

            test_function!(
                CharacterLengthFunc::new(),
                vec![ColumnarValue::Scalar(ScalarValue::LargeUtf8($INPUT))],
                $EXPECTED,
                i64,
                Int64,
                Int64Array
            );

            test_function!(
                CharacterLengthFunc::new(),
                vec![ColumnarValue::Scalar(ScalarValue::Utf8View($INPUT))],
                $EXPECTED,
                i32,
                Int32,
                Int32Array
            );
        };
    }

    #[test]
    fn test_functions() -> Result<()> {
        #[cfg(feature = "unicode_expressions")]
        {
            test_character_length!(Some(String::from("chars")), Ok(Some(5)));
            test_character_length!(Some(String::from("josé")), Ok(Some(4)));
            // test long strings (more than 12 bytes for StringView)
            test_character_length!(Some(String::from("joséjoséjoséjosé")), Ok(Some(16)));
            test_character_length!(Some(String::from("")), Ok(Some(0)));
            test_character_length!(None, Ok(None));
        }

        #[cfg(not(feature = "unicode_expressions"))]
        test_function!(
            CharacterLengthFunc::new(),
            &[ColumnarValue::Scalar(ScalarValue::Utf8(Some(String::from("josé"))))],
            internal_err!(
                "function character_length requires compilation with feature flag: unicode_expressions."
            ),
            i32,
            Int32,
            Int32Array
        );

        Ok(())
    }
}
