p = 'tools/window9a_run.py'
s = open(p, encoding='utf-8').read()
old = """    tf = s.get('target_features') if isinstance(s.get('target_features'), dict) else {}
    if any(tf.get(k) is not True for k in RAPIER_TARGET_FEATURES):
        flag('V4', f'target_features {s.get("target_features")}, want avx2 fma bmi2 all true')
"""
new = """    if s.get('target_features') != {k: True for k in RAPIER_TARGET_FEATURES}:
        flag('V4', f'target_features {s.get("target_features")}, want exactly avx2 fma bmi2, all true')
"""
assert s.count(old) == 1
s = s.replace(old, new)
open(p, 'w', encoding='utf-8', newline='\n').write(s)
print('ok')
