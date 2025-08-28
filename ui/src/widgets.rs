use xilem::WidgetView;
use xilem::style::{Padding, Style};
use xilem::view::{SizedBox, sized_box};

pub fn margin<State, Action, V>(inner: V, margin: impl Into<Padding>) -> SizedBox<V, State, Action>
where
    V: WidgetView<State, Action>,
{
    sized_box(inner).padding(margin)
}
