# SILKSONG_PATH := "C:/Program Files (x86)/Steam/steamapps/common/Hollow Knight Silksong/Hollow Knight Silksong_Data"
SILKSONG_PATH := "/home/jakob/.local/share/Steam/steamapps/common/Hollow Knight Silksong"

enemies:
    cargo run -r -p uniscan --bin uniscan -- "{{SILKSONG_PATH}}" HealthManager '{ \
        file: ._file, \
        path: go|path, \
        fsm: [go | fsm .fsm.name], \
        enemySize, enemyType, hp, \
        journal: go|components("MonoBehaviour") | select(script_name | startswith("EnemyDeathEffects")) | .journalRecord | maybe(deref .m_Name) \
    }' > out/enemies.json

fsms:
    cargo run -r -p uniscan -- "{{SILKSONG_PATH}}" HealthManager '{_file, name: go|path, fsms: [go|scripts("PlayMakerFSM") .fsm.name ] }' > out/fsms.json

by-journal:
    cat ./out/enemies.json | jq -s 'reduce (.[]|select(.journal!=null)) as $item ({}; .[$item.journal] += [$item]) | map_values(sort_by(.path) | first | { file, path })' > out/by-journal.json

preloads:
    cat ./out/enemies.json | jq -s 'reduce (.[]|select(.journal!=null)) as $item ({}; .[$item.file] += [$item]) | map_values(group_by(.journal) | map_values(sort_by(.path) | first .path)  ) | with_entries(select(.key | contains("scenes_scenes_scenes")))' > out/preloads.json
