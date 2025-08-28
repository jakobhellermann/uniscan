def go: .m_GameObject | deref;
def name: .m_GameObject | deref | .m_Name;

def script_name: .m_Script | deref | .m_ClassName;

def components: .m_Component[].component;
def components(class_id): components | select(.class_id == class_id) | deref;
def scripts: components("MonoBehaviour");
def scripts(name): components("MonoBehaviour") | select(script_name == name);

def fsm: scripts("PlayMakerFSM");

def depth1: del(.[]?[]?);
def depth2: del(.[]?[]?[]?);
def depth3: del(.[]?[]?[]?[]?);
