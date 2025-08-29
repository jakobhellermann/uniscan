def maybe(f): if . != null then f else null end;
def nonnull: select(. != null);
def filterkeys(text): with_entries(select(.key | contains(text)));

def go: .m_GameObject | deref;
def name: if .m_Name != "" then .m_Name else go | .m_Name end;

# monobehaviour
def script_name: .m_Script | deref | .m_ClassName;

# game object
def components: .m_Component[].component;
def components(class_id): components | select(.class_id == class_id) | deref;
def scripts: components("MonoBehaviour");
def transform: components("Transform");
def scripts(name): components("MonoBehaviour") | select(script_name == name);

# transforms
def parent: transform | .m_Father | maybe(deref) | maybe(go);
def path_components: parent as $parent |
    if $parent == null then [name]
    else ($parent | path_components) + [name]
    end;
def path: parent as $parent |
    if $parent == null then name
    else ($parent | path) + "/" + name
    end;

def fsm: scripts("PlayMakerFSM");

def depth1: del(.[]?[]?);
def depth2: del(.[]?[]?[]?);
def depth3: del(.[]?[]?[]?[]?);
