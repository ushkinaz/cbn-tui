# Flatten CBN-style copy-from inheritance for .data[]
# Assumptions:
# - no mod layering/self-override chains
# - inheritance is resolved by id
#
# Usage:
#   jq -f scripts/flatten-copy-from.jq _test/all.json
#
# Output:
#   flattened .data array

def trunc0:
  if . >= 0 then floor else ceil end;

def parse_numbered_unit:
  capture("^\\s*(?<num>-?(?:[0-9]+(?:\\.[0-9]+)?|\\.[0-9]+))\\s*(?<unit>.+?)\\s*$")?;

def arr_eq($a; $b):
  if ($a | type) != ($b | type) then false
  elif ($a | type) == "array" then
    ($a | length) == ($b | length)
    and all(range(0; $a | length); arr_eq($a[.]; $b[.]))
  else
    $a == $b
  end;

def to_dmg_array:
  if . == null then []
  elif (type == "array") then .
  elif (type == "object" and has("damage_type") and has("amount")) then [.]
  else []
  end;

def as_array:
  if type == "array" then . else [] end;

def merge_relative_damage($base; $delta):
  reduce ($delta | to_dmg_array)[] as $d
    ($base | to_dmg_array;
      (map(.damage_type) | index($d.damage_type)) as $i
      | if $i == null then . + [$d]
        else .[$i].amount = ((.[$i].amount // 0) + ($d.amount // 0))
        end
    );

def merge_proportional_damage($base; $factor):
  reduce ($factor | to_dmg_array)[] as $d
    ($base | to_dmg_array;
      (map(.damage_type) | index($d.damage_type)) as $i
      | if $i == null then .
        else .[$i].amount = ((.[$i].amount // 0) * ($d.amount // 1))
        end
    );

def apply_relative:
  reduce ((.relative // {}) | keys[]) as $k
    (.;
      . as $root
      | if ($root.relative[$k] | type) == "number" then
          if ($root[$k] | type) == "number" then
            .[$k] = (($root[$k] // 0) + $root.relative[$k])
          elif ($root[$k] | type) == "string" then
            ($root[$k] | parse_numbered_unit) as $m
            | if $m == null then .
              else .[$k] = (((($m.num | tonumber) + $root.relative[$k]) | tostring) + " " + $m.unit)
              end
          else
            .[$k] = (($root[$k] // 0) + $root.relative[$k])
          end
        elif (($k == "damage" or $k == "ranged_damage") and ($root[$k] != null)) then
          .[$k] = merge_relative_damage($root[$k]; $root.relative[$k])
        elif ($k == "armor" and $root.type == "MONSTER" and ($root[$k] | type) == "object") then
          .[$k] = ($root[$k] // {})
          | reduce (($root.relative[$k] // {}) | keys[]) as $k2
              (.;
                .[$k][$k2] = ((.[$k][$k2] // 0) + ($root.relative[$k][$k2] // 0))
              )
        elif ($k == "qualities" and ($root[$k] | type) == "array") then
          .[$k] = (
            reduce (($root.relative[$k] // []))[] as $q
              (($root[$k] // []);
                (map(.[0]) | index($q[0])) as $i
                | if $i == null then .
                  else .[$i][1] = ((.[$i][1] // 0) + ($q[1] // 0))
                  end
              )
          )
        else .
        end
    )
  | del(.relative);

def apply_proportional:
  reduce ((.proportional // {}) | keys[]) as $k
    (.;
      . as $root
      | if ($root.proportional[$k] | type) == "number" then
          if ($root[$k] | type) == "number" then
            .[$k] = ((($root[$k] // 0) * $root.proportional[$k]) | trunc0)
          elif ($root[$k] | type) == "string" then
            ($root[$k] | parse_numbered_unit) as $m
            | if $m == null then .
              else .[$k] = (((($m.num | tonumber) * $root.proportional[$k]) | tostring) + " " + $m.unit)
              end
          elif ($k == "attack_cost" and ($root[$k] == null)) then
            .[$k] = ((100 * $root.proportional[$k]) | trunc0)
          else
            .[$k] = ((($root[$k] // 0) * $root.proportional[$k]) | trunc0)
          end
        elif (($k == "damage" or $k == "ranged_damage") and ($root[$k] != null)) then
          .[$k] = merge_proportional_damage($root[$k]; $root.proportional[$k])
        elif ($k == "armor" and $root.type == "MONSTER" and ($root[$k] | type) == "object") then
          .[$k] = ($root[$k] // {})
          | reduce (($root.proportional[$k] // {}) | keys[]) as $k2
              (.;
                .[$k][$k2] =
                  (((.[$k][$k2] // 0) * ($root.proportional[$k][$k2] // 1)) | trunc0)
              )
        else .
        end
    )
  | del(.proportional);

def apply_extend:
  reduce ((.extend // {}) | keys[]) as $k
    (.;
      . as $root
      | if ($root.extend[$k] | type) == "array" then
          if $k == "flags" then
            .[$k] = (($root[$k] | as_array) + (($root.extend[$k]) - ($root[$k] | as_array)))
          else
            .[$k] = (($root[$k] | as_array) + $root.extend[$k])
          end
        else .
        end
    )
  | del(.extend);

def apply_delete:
  reduce ((.delete // {}) | keys[]) as $k
    (.;
      . as $root
      | if ($root.delete[$k] | type) == "array" then
          .[$k] = (
            ($root[$k] | as_array)
            | map(select(. as $x | (($root.delete[$k] | any(arr_eq(.; $x))) | not)))
          )
        else
          del(.[$k])
        end
    )
  | del(.delete);

def apply_special_merges($parent_props; $obj; $ret):
  ($ret
    | if ($parent_props.vitamins | type) == "array" and ($obj.vitamins | type) == "array" then
        .vitamins = [
          ($parent_props.vitamins[] | select(([ $obj.vitamins[] | .[0] ] | index(.[0])) == null)),
          ($obj.vitamins[])
        ]
      else .
      end
    | if $obj.type == "vehicle" and ($parent_props.parts | type) == "array" and ($obj.parts | type) == "array" then
        .parts = (($parent_props.parts | as_array) + ($obj.parts | as_array))
      else .
      end
  );

def flatten($idx; $id; $stack):
  if ($stack | index($id)) != null then
    ($idx[$id])
  else
    ($idx[$id]) as $obj
    | if $obj == null then null
      else ($obj["copy-from"] // null) as $pid
      | if $pid == null or $idx[$pid] == null or $pid == $id then
          $obj
        else
          (flatten($idx; $pid; $stack + [$id])) as $parent
          | ($parent | del(.abstract)) as $parent_props
          | ($parent_props + $obj) as $ret
          | apply_special_merges($parent_props; $obj; $ret)
          | apply_relative
          | apply_proportional
          | apply_extend
          | apply_delete
        end
      end
  end;

.data as $rows
| ($rows | map(select((.id | type) == "string") | { key: .id, value: . }) | from_entries) as $idx
| $rows
| map(
    if (.id | type) == "string" then flatten($idx; .id; []) else . end
  )
