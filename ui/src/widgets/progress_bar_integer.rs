use xilem::core::{MessageContext, MessageResult, Mut, View, ViewMarker};
use xilem::{Pod, ViewCtx};

/// A view which displays a progress bar.
///
/// This can be for showing progress of a task or a download.
pub fn progress_bar(progress: Option<f64>) -> ProgressBar {
    ProgressBar { progress }
}

/// The [`View`] created by [`progress_bar`].
#[must_use = "View values do nothing unless provided to Xilem."]
pub struct ProgressBar {
    progress: Option<f64>,
}

impl ViewMarker for ProgressBar {}
impl<State, Action> View<State, Action, ViewCtx> for ProgressBar {
    type Element = Pod<widget::ProgressBar>;
    type ViewState = ();

    fn build(&self, ctx: &mut ViewCtx, _: &mut State) -> (Self::Element, Self::ViewState) {
        ctx.with_leaf_action_widget(|ctx| ctx.create_pod(widget::ProgressBar::new(self.progress)))
    }

    fn rebuild(
        &self,
        prev: &Self,
        (): &mut Self::ViewState,
        _ctx: &mut ViewCtx,
        mut element: Mut<'_, Self::Element>,
        _: &mut State,
    ) {
        if prev.progress != self.progress {
            widget::ProgressBar::set_progress(&mut element, self.progress);
        }
    }

    fn teardown(
        &self,
        (): &mut Self::ViewState,
        ctx: &mut ViewCtx,
        element: Mut<'_, Self::Element>,
    ) {
        ctx.teardown_leaf(element);
    }

    fn message(
        &self,
        (): &mut Self::ViewState,
        message: &mut MessageContext,
        _element: Mut<'_, Self::Element>,
        _app_state: &mut State,
    ) -> MessageResult<Action> {
        tracing::error!(
            ?message,
            "Message arrived in ProgressBar::message, but ProgressBar doesn't consume any messages, this is a bug"
        );
        MessageResult::Stale
    }
}

pub mod widget {

    use std::any::TypeId;

    use masonry::accesskit::{Node, Role};
    use masonry::core::{NewWidget, Properties};
    use masonry::util::{fill, stroke};
    use masonry::vello::Scene;
    use masonry::vello::kurbo::{Point, Size};
    use tracing::{Span, trace_span};

    use masonry::core::{
        AccessCtx, ArcStr, BoxConstraints, ChildrenIds, LayoutCtx, NoAction, PaintCtx,
        PropertiesMut, PropertiesRef, RegisterCtx, Update, UpdateCtx, Widget, WidgetId, WidgetMut,
        WidgetPod,
    };
    use masonry::properties::{
        Background, BarColor, BorderColor, BorderWidth, CornerRadius, LineBreaking,
    };
    use masonry::widgets::Label;

    // TODO - NaN probably shouldn't be a meaningful value in our API.

    /// A progress bar.
    pub struct ProgressBar {
        /// A value in the range `[0, 1]` inclusive, where 0 is 0% and 1 is 100% complete.
        ///
        /// `None` variant can be used to show a progress bar without a percentage.
        /// It is also used if an invalid float (outside of [0, 1]) is passed.
        progress: Option<f64>,
        label: WidgetPod<Label>,
    }

    impl ProgressBar {
        /// Create a new `ProgressBar`.
        ///
        /// The progress value will be clamped to [0, 1].
        ///
        /// A `None` value (or NaN) will show an indeterminate progress bar.
        pub fn new(progress: Option<f64>) -> Self {
            let progress = clamp_progress(progress);
            let label_props = Properties::one(LineBreaking::Overflow);
            let label =
                NewWidget::new_with_props(Label::new(Self::value(progress)), label_props).to_pod();
            Self { progress, label }
        }

        fn value_accessibility(&self) -> Box<str> {
            if let Some(value) = self.progress {
                format!("{:.0}%", value * 100.).into()
            } else {
                "progress unspecified".into()
            }
        }

        fn value(progress: Option<f64>) -> ArcStr {
            if let Some(value) = progress {
                format!("{:.0}%", value * 100.).into()
            } else {
                "".into()
            }
        }
    }

    // --- MARK: WIDGETMUT
    impl ProgressBar {
        /// Set the progress displayed by the bar.
        ///
        /// The progress value will be clamped to [0, 1].
        ///
        /// A `None` value (or NaN) will show an indeterminate progress bar.
        pub fn set_progress(this: &mut WidgetMut<'_, Self>, progress: Option<f64>) {
            let progress = clamp_progress(progress);
            let progress_changed = this.widget.progress != progress;
            if progress_changed {
                this.widget.progress = progress;
                let mut label = this.ctx.get_mut(&mut this.widget.label);
                Label::set_text(&mut label, Self::value(progress));
            }
            this.ctx.request_layout();
            this.ctx.request_render();
        }
    }

    /// Helper to ensure progress is either a number between [0, 1] inclusive, or `None`.
    ///
    /// NaNs are converted to `None`.
    fn clamp_progress(progress: Option<f64>) -> Option<f64> {
        let progress = progress?;
        if progress.is_nan() {
            None
        } else {
            Some(progress.clamp(0., 1.))
        }
    }

    // --- MARK: IMPL WIDGET
    impl Widget for ProgressBar {
        type Action = NoAction;

        fn register_children(&mut self, ctx: &mut RegisterCtx<'_>) {
            ctx.register_child(&mut self.label);
        }

        fn property_changed(&mut self, ctx: &mut UpdateCtx<'_>, property_type: TypeId) {
            BorderWidth::prop_changed(ctx, property_type);
            CornerRadius::prop_changed(ctx, property_type);
            Background::prop_changed(ctx, property_type);
            BarColor::prop_changed(ctx, property_type);
            BorderColor::prop_changed(ctx, property_type);
        }

        fn update(
            &mut self,
            _ctx: &mut UpdateCtx<'_>,
            _props: &mut PropertiesMut<'_>,
            _event: &Update,
        ) {
        }

        fn layout(
            &mut self,
            ctx: &mut LayoutCtx<'_>,
            _props: &mut PropertiesMut<'_>,
            bc: &BoxConstraints,
        ) -> Size {
            const DEFAULT_WIDTH: f64 = 400.;
            // TODO: Clearer constraints here
            let label_size = ctx.run_layout(&mut self.label, &bc.loosen());
            let desired_size = Size::new(
                DEFAULT_WIDTH.max(label_size.width),
                masonry::theme::BASIC_WIDGET_HEIGHT.max(label_size.height),
            );
            let final_size = bc.constrain(desired_size);

            // center text
            let text_pos = Point::new(
                ((final_size.width - label_size.width) * 0.5).max(0.),
                ((final_size.height - label_size.height) * 0.5).max(0.),
            );
            ctx.place_child(&mut self.label, text_pos);
            final_size
        }

        fn paint(&mut self, ctx: &mut PaintCtx<'_>, props: &PropertiesRef<'_>, scene: &mut Scene) {
            let border_width = props.get::<BorderWidth>();
            let border_radius = props.get::<CornerRadius>();
            let bg = props.get::<Background>();
            let bar_color = props.get::<BarColor>();
            let border_color = props.get::<BorderColor>();

            let bg_rect = border_width.bg_rect(ctx.size(), border_radius);
            let border_rect = border_width.border_rect(ctx.size(), border_radius);

            let progress_rect_size = Size::new(
                ctx.size().width * self.progress.unwrap_or(1.),
                ctx.size().height,
            );
            let progress_rect = border_width.bg_rect(progress_rect_size, border_radius);

            let brush = bg.get_peniko_brush_for_rect(bg_rect.rect());
            fill(scene, &bg_rect, &brush);
            fill(scene, &progress_rect, bar_color.0);

            stroke(scene, &border_rect, border_color.color, border_width.width);
        }

        fn accessibility_role(&self) -> Role {
            Role::ProgressIndicator
        }

        fn accessibility(
            &mut self,
            _ctx: &mut AccessCtx<'_>,
            _props: &PropertiesRef<'_>,
            node: &mut Node,
        ) {
            node.set_min_numeric_value(0.0);
            node.set_max_numeric_value(1.0);
            if let Some(value) = self.progress {
                node.set_numeric_value(value);
            }
        }

        fn children_ids(&self) -> ChildrenIds {
            ChildrenIds::from_slice(&[self.label.id()])
        }

        fn make_trace_span(&self, id: WidgetId) -> Span {
            trace_span!("ProgressBar", id = id.trace())
        }

        fn get_debug_text(&self) -> Option<String> {
            Some(self.value_accessibility().into())
        }
    }
}
