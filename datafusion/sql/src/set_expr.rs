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

use crate::planner::{ContextProvider, PlannerContext, SqlToRel};
use datafusion_common::{
    not_impl_err, plan_err, DataFusionError, Diagnostic, Result, Span,
};
use datafusion_expr::{LogicalPlan, LogicalPlanBuilder};
use sqlparser::ast::{SetExpr, SetOperator, SetQuantifier, Spanned};

impl<S: ContextProvider> SqlToRel<'_, S> {
    #[cfg_attr(feature = "recursive_protection", recursive::recursive)]
    pub(super) fn set_expr_to_plan(
        &self,
        set_expr: SetExpr,
        planner_context: &mut PlannerContext,
    ) -> Result<LogicalPlan> {
        let set_expr_span = Span::try_from_sqlparser_span(set_expr.span());
        match set_expr {
            SetExpr::Select(s) => self.select_to_plan(*s, None, planner_context),
            SetExpr::Values(v) => self.sql_values_to_plan(v, planner_context),
            SetExpr::SetOperation {
                op,
                left,
                right,
                set_quantifier,
            } => {
                let left_span = Span::try_from_sqlparser_span(left.span());
                let right_span = Span::try_from_sqlparser_span(right.span());
                let left_plan = self.set_expr_to_plan(*left, planner_context);
                let right_plan = self.set_expr_to_plan(*right, planner_context);
                let (left_plan, right_plan) = match (left_plan, right_plan) {
                    (Ok(left_plan), Ok(right_plan)) => (left_plan, right_plan),
                    (Err(left_err), Err(right_err)) => {
                        return Err(DataFusionError::Collection(vec![
                            left_err, right_err,
                        ]));
                    }
                    (Err(err), _) | (_, Err(err)) => {
                        return Err(err);
                    }
                };
                if !(set_quantifier == SetQuantifier::ByName
                    || set_quantifier == SetQuantifier::AllByName)
                {
                    self.validate_set_expr_num_of_columns(
                        op,
                        left_span,
                        right_span,
                        &left_plan,
                        &right_plan,
                        set_expr_span,
                    )?;
                }
                self.set_operation_to_plan(op, left_plan, right_plan, set_quantifier)
            }
            SetExpr::Query(q) => self.query_to_plan(*q, planner_context),
            _ => not_impl_err!("Query {set_expr} not implemented yet"),
        }
    }

    pub(super) fn is_union_all(set_quantifier: SetQuantifier) -> Result<bool> {
        match set_quantifier {
            SetQuantifier::All | SetQuantifier::AllByName => Ok(true),
            SetQuantifier::Distinct
            | SetQuantifier::ByName
            | SetQuantifier::DistinctByName
            | SetQuantifier::None => Ok(false),
        }
    }

    fn validate_set_expr_num_of_columns(
        &self,
        op: SetOperator,
        left_span: Option<Span>,
        right_span: Option<Span>,
        left_plan: &LogicalPlan,
        right_plan: &LogicalPlan,
        set_expr_span: Option<Span>,
    ) -> Result<()> {
        if left_plan.schema().fields().len() == right_plan.schema().fields().len() {
            return Ok(());
        }
        let diagnostic = Diagnostic::new_error(
            format!("{} queries have different number of columns", op),
            set_expr_span,
        )
        .with_note(
            format!("this side has {} fields", left_plan.schema().fields().len()),
            left_span,
        )
        .with_note(
            format!(
                "this side has {} fields",
                right_plan.schema().fields().len()
            ),
            right_span,
        );
        plan_err!("{} queries have different number of columns", op; diagnostic =diagnostic)
    }

    pub(super) fn set_operation_to_plan(
        &self,
        op: SetOperator,
        left_plan: LogicalPlan,
        right_plan: LogicalPlan,
        set_quantifier: SetQuantifier,
    ) -> Result<LogicalPlan> {
        match (op, set_quantifier) {
            (SetOperator::Union, SetQuantifier::All) => {
                LogicalPlanBuilder::from(left_plan)
                    .union(right_plan)?
                    .build()
            }
            (SetOperator::Union, SetQuantifier::AllByName) => {
                LogicalPlanBuilder::from(left_plan)
                    .union_by_name(right_plan)?
                    .build()
            }
            (SetOperator::Union, SetQuantifier::Distinct | SetQuantifier::None) => {
                LogicalPlanBuilder::from(left_plan)
                    .union_distinct(right_plan)?
                    .build()
            }
            (
                SetOperator::Union,
                SetQuantifier::ByName | SetQuantifier::DistinctByName,
            ) => LogicalPlanBuilder::from(left_plan)
                .union_by_name_distinct(right_plan)?
                .build(),
            (SetOperator::Intersect, SetQuantifier::All) => {
                LogicalPlanBuilder::intersect(left_plan, right_plan, true)
            }
            (SetOperator::Intersect, SetQuantifier::Distinct | SetQuantifier::None) => {
                LogicalPlanBuilder::intersect(left_plan, right_plan, false)
            }
            (SetOperator::Except, SetQuantifier::All) => {
                LogicalPlanBuilder::except(left_plan, right_plan, true)
            }
            (SetOperator::Except, SetQuantifier::Distinct | SetQuantifier::None) => {
                LogicalPlanBuilder::except(left_plan, right_plan, false)
            }
            (op, quantifier) => {
                not_impl_err!("{op} {quantifier} not implemented")
            }
        }
    }
}
