#!/usr/bin/env python3
"""DJI's own on-board library as the oracle for `s1-duml`'s encoders.

The robot carries the Scratch/Python runtime under `/system/firm/dji_scratch/src/robomaster/`;
`rm_module.py` builds every command the body accepts and `duss_event_msg.py` packs it, and both
run unchanged on a Mac. This script runs that packer for every UART-bound command with sender
`scratch_script_id` (0xC9) and seq 0, and prints `name hex` — the expected values in
`s1-duml/tests/dji_oracle.rs`. The library is DJI's and stays out of git: pull it from a rooted
robot with `adb pull /system/firm/dji_scratch <lab>/stock/dji_scratch` and point here at it.

Usage: scripts/s1/dji-oracle.py [<path to .../dji_scratch/src/robomaster>] [--json out.json]
Default path: ~/Development/RoboMaster-S1/out/2026-10-30-poc2/stock/dji_scratch/src/robomaster
"""
import sys, types, logging, json, os
args = [a for a in sys.argv[1:] if not a.startswith("--")]
SRC = args[0] if args else os.path.expanduser("~/Development/RoboMaster-S1/out/2026-10-30-poc2/stock/dji_scratch/src/robomaster")
if not os.path.isdir(SRC):
    sys.exit(f"no DJI library at {SRC}; pull /system/firm/dji_scratch from the robot first")
sys.path.insert(0, SRC)
rm_log = types.ModuleType("rm_log"); rm_log.dji_scratch_logger_get = lambda: logging.getLogger("x"); sys.modules["rm_log"] = rm_log
import duss_event_msg as dem, rm_define as D, duml_cmdset as C
out = {}
def msg(receiver, module, cmdset=C.DUSS_MB_CMDSET_RM):
    m = dem.EventMsg(D.scratch_script_id); m.set_default_receiver(receiver); m.set_default_moduleid(module); m.set_default_cmdset(cmdset); m.init(); m.seq_num = 0; return m
def emit(name, m):
    out[name] = m.pack().hex()
# chassis (receiver 306 = 0xC3)
for mode in (0, 1, 2, 3):
    m = msg(D.chassis_id, D.chassis_id); m.append('mode','uint8',mode); m.cmd_id = C.DUSS_MB_CMD_RM_WORK_MODE_SET; emit(f"chassis_workmode_{mode}", m)
m = msg(D.chassis_id, D.chassis_id); [m.append(f'speed{i}','int16',v) for i, v in enumerate((100,-200,-300,400),1)]; m.cmd_id = C.DUSS_MB_CMD_RM_WHEEL_SPEED_SET; emit("chassis_wheel_speed_100_m200_m300_400", m)
m = msg(D.chassis_id, D.chassis_id); m.append('speed_x','float',0.2); m.append('speed_y','float',-0.1); m.append('speed_z','float',30.0); m.cmd_id = C.DUSS_MB_CMD_RM_SPEED_SET; m.cmd_type = C.NO_ACK_TYPE; emit("chassis_speed_0.2_-0.1_30", m)
m = msg(D.chassis_id, D.chassis_id); m.append('speed_x','float',0.0); m.append('speed_y','float',0.0); m.append('angle','float',45.0); m.cmd_id = C.DUSS_MB_CMD_RM_FOLLOW_MODE_SET; m.cmd_type = C.NO_ACK_TYPE; emit("chassis_follow_0_0_45", m)
for f in (0, 1, 2):
    m = msg(D.chassis_id, D.chassis_id); m.append('enable','uint8',f); m.cmd_id = C.DUSS_MB_CMD_RM_SPEED_MODE_SET; emit(f"chassis_stick_overlay_{f}", m)
m = msg(D.chassis_id, D.chassis_id); task_ctrl = C.TASK_FREQ_10Hz << 2 | C.TASK_CTRL_START
m.append('task_id','uint8',7); m.append('task_ctrl','uint8',task_ctrl); m.append('ctrl_mode','uint8',0); m.append('axis_mode','uint8',0); m.append('pos_x','int16',100); m.append('pos_y','int16',-50); m.append('angle_yaw','int16',900); m.append('vel_xy_max','uint8',50); m.append('agl_omg_max','uint16',300); m.cmd_id = C.DUSS_MB_CMD_RM_POSITION_SET; emit("chassis_position_task7_x100_ym50_yaw900_v50_w300", m)
m = msg(D.chassis_id, D.chassis_id); m.append('task_id','uint8',7); m.append('task_ctrl','uint8',C.TASK_CTRL_STOP); m.append('ctrl_mode','uint8',0); m.append('axis_mode','uint8',0); m.append('pos_x','int16',0); m.append('pos_y','int16',0); m.append('angle_yaw','int16',0); m.cmd_id = C.DUSS_MB_CMD_RM_POSITION_SET; emit("chassis_position_stop_task7", m)
m = msg(D.chassis_id, D.chassis_id); m.append('mode','uint8',0x3f); [m.append(f'p{i}','uint16',500) for i in range(6)]; m.cmd_id = C.DUSS_MB_CMD_RM_SET_CHASSIS_PWM_VALUE; emit("chassis_pwm_value_all_500", m)
m = msg(D.chassis_id, D.chassis_id); m.append('mode','uint8',0x3f); [m.append(f'p{i}','uint16',1000) for i in range(6)]; m.cmd_id = C.DUSS_MB_CMD_RM_SET_CHASSIS_PWM_FREQ; emit("chassis_pwm_freq_all_1000", m)
# gimbal (receiver 400 = 0x04)
for mode in (0, 1, 2):
    m = msg(D.gimbal_id, D.gimbal_id, C.DUSS_MB_CMDSET_GIMBAL); m.append('mode','uint8',mode); m.append('cmd','uint8',0); m.cmd_id = C.DUSS_MB_CMD_GIMBAL_SET_MODE; emit(f"gimbal_workmode_{mode}", m)
m = msg(D.gimbal_id, D.gimbal_id, C.DUSS_MB_CMDSET_GIMBAL); m.append('task_id','uint8',7); m.append('task_ctrl','uint8',task_ctrl); m.append('axis_maskbit','uint8',D.gimbal_axis_pitch_yaw_maskbit); m.append('max_yaw_accel','uint16',100); m.append('max_roll_accel','uint16',0); m.append('max_pitch_accel','uint16',100); m.cmd_set = C.DUSS_MB_CMDSET_RM; m.cmd_id = C.DUSS_MB_CMD_RM_GIMBAL_RESET_POSITION_SET; emit("gimbal_recenter_task7_yaw100_pitch100", m)
m = msg(D.gimbal_id, D.gimbal_id, C.DUSS_MB_CMDSET_GIMBAL); m.append('task_id','uint8',7); m.append('task_ctrl','uint8',task_ctrl); ctrl = ((D.gimbal_coodrdinate_cur & 0x07) << 3) | D.gimbal_axis_pitch_yaw_maskbit
m.append('ctrl','uint8',ctrl); m.append('yaw_degree','int16',300); m.append('roll_degree','int16',0); m.append('pitch_degree','int16',-100); m.append('deviation','int16',0); m.append('yaw_accel','uint16',120); m.append('roll_accel','uint16',0); m.append('pitch_accel','uint16',120); m.cmd_set = C.DUSS_MB_CMDSET_RM; m.cmd_id = C.DUSS_MB_CMD_RM_GIMBAL_DEGREE_SET; emit("gimbal_degree_task7_cur_yaw300_pitchm100_acc120", m)
m = msg(D.gimbal_id, D.gimbal_id, C.DUSS_MB_CMDSET_GIMBAL); m.append('yaw_accel','int16',300); m.append('roll_accel','int16',0); m.append('pitch_accel','int16',-150); m.append('contral','uint8',0xdc); m.cmd_id = C.DUSS_MB_CMD_GIMBAL_EXT_CTRL_ACCEL; m.cmd_type = C.NO_ACK_TYPE; emit("gimbal_speed_yaw300_pitchm150_ctrl_dc", m)
for name, code in (("suspend", 0x2ab5), ("resume", 0x7ef2)):
    m = msg(D.gimbal_id, D.gimbal_id, C.DUSS_MB_CMDSET_GIMBAL); m.append('cmd','uint16',code); m.cmd_id = C.DUSS_MB_CMD_GIMBAL_SUSPEND_RESUME; m.cmd_type = C.REQ_PKG_TYPE | C.NEED_ACK_NO_FINISH_TYPE; emit(f"gimbal_{name}", m)
m = msg(D.gimbal_id, D.gimbal_id, C.DUSS_MB_CMDSET_GIMBAL); m.append('cmd','uint8',C.DUSS_MB_CMD_GIMBAL_PUSH_POSITION); m.append('other','uint8',0xff); m.cmd_id = C.DUSS_MB_CMD_GIMBAL_DEGREE_INFO_SUBSCRIPTION; m.cmd_type = C.REQ_PKG_TYPE | C.NEED_ACK_TYPE; emit("gimbal_sub_position_on", m)
# gun (fire goes to hdvt 0x09; led to gun 0x17)
m = msg(D.gun_id, D.gun_id); m.append('cmd','uint8',((D.single_fire_mode << 4) & 0xf0) + 1); m.cmd_id = C.DUSS_MB_CMD_RM_SHOOT_CMD; m.receiver = D.hdvt_uav_id; emit("gun_fire_single_1_to_hdvt", m)
# `GunCtrl.set_led(mode)` → `Gun.set_led(mode, 7)`: control 7 with the effect code of rm_define (on 0, off 1, breath 2, flash 3, marquee 4)
for mode in range(5):
    m = msg(D.gun_id, D.gun_id); m.append('ctrl_mode','uint8',((7 << 4) & 0xf0) + mode); m.append('r','uint8',255); m.append('g','uint8',255); m.append('b','uint8',255); m.append('times','uint8',1); m.append('t1','uint16',100); m.append('t2','uint16',100); m.cmd_id = C.DUSS_MB_CMD_RM_GUN_LED_SET; emit(f"gun_led_ctrl7_mode{mode}", m)
m = msg(D.gun_id, D.gun_id); m.append('ctrl_mode','uint8',((1 << 4) & 0xf0) + 0); m.append('r','uint8',255); m.append('g','uint8',255); m.append('b','uint8',255); m.append('times','uint8',1); m.append('t1','uint16',100); m.append('t2','uint16',100); m.cmd_id = C.DUSS_MB_CMD_RM_GUN_LED_SET; emit("gun_led_ctrl1_mode0", m)
# armour sensitivity (to hdvt 0x09)
m = msg(D.armor_id, D.armor_id); k = 1.0
for n, v in (('set_type',0x3f),('voice_energy_entry',500),('voice_energy_exit',300),('voice_len_max',50),('voice_len_min',13),('voice_silence_wait_limit',6),('voice_peak_count_min',1),('voice_multi_peak_min',int(160*k)),('voice_multi_peak_ave',int(180*k)),('voice_multi_final_peak',int(200*k))):
    m.append(n, 'uint8' if n == 'set_type' else 'uint16', v)
m.receiver = D.hdvt_uav_id; m.cmd_id = C.DUSS_MB_CMD_RM_ARMOR_VOICE_PARAMS_SET; emit("armor_sensitivity_k1_to_hdvt", m)
# tank / robot mode (to hdvt 0x09)
for mode in (1, 2):
    m = msg(D.hdvt_uav_id, D.system_id); m.append('mode','uint8',mode); m.cmd_id = C.DUSS_MB_CMD_RM_SET_TANK_WORK_MODE; emit(f"tank_workmode_{mode}_to_hdvt", m)
# virtual bus (DDS) as the Scratch runtime does it: node (9,6)=0xC9
node = (9 & 0x1f) | ((6 << 5) & 0xe0)
m = msg(D.hdvt_uav_id, D.system_id, C.DUSS_MB_CMDSET_VIRTUAL_BUS); m.append('node_id','uint8',node); m.append('version','uint32',0x3000000); m.cmd_id = C.DUSS_MB_CMD_VBUS_ADD_NODE; emit("vbus_add_node_c9_to_hdvt", m)
m = msg(D.hdvt_uav_id, D.system_id, C.DUSS_MB_CMDSET_VIRTUAL_BUS); m.append('node_id','uint8',node); m.cmd_id = C.DUSS_MB_CMD_VBUS_NODE_RESET; emit("vbus_node_reset_c9_to_hdvt", m)
m = msg(D.hdvt_uav_id, D.system_id, C.DUSS_MB_CMDSET_VIRTUAL_BUS); m.append('node_id','uint8',node); m.append('msg_id','uint8',0); m.append('sub_config','uint8',0); m.append('mode','uint8',0); uuids = [0x6862229f]; m.append('uuid_num','uint8',len(uuids))
for i, u in enumerate(uuids): m.append(f'uuid_{i}_l','uint32',u); m.append(f'uuid_{i}_h','uint32',0x20009)
m.append('freq','uint16',20); m.cmd_id = C.DUSS_MB_CMD_VBUS_ADD_MSG; emit("vbus_add_msg_battery_20hz_to_hdvt", m)
# system LEDs as the stock app writes them (captured 2026-10-30: 0x3f/0x32 to host 0x18 = armor_id,
# no ack): effect, led_mask u16, r g b, repeat, t1 u16, t2 u16, comp_mask u16. Effects: 1 solid,
# 2 breath (both captured); 3 flash, 4 marquee by the library's `effect_*` numbering, seen on the robot.
for name, effect, rgb, t in (("solid_red", 1, (255,0,0), (0,0)), ("breath_blue", 2, (0,0,255), (500,500)), ("flash_green", 3, (0,255,0), (250,250)), ("marquee_white", 4, (255,255,255), (100,100)), ("off", 0, (0,0,0), (0,0))):
    m = msg(D.hdvt_uav_id, D.system_id); m.sender = D.hdvt_uav_id; m.receiver = D.armor_id; m.cmd_type = C.NO_ACK_TYPE
    m.append('effect','uint8',effect); m.append('led_mask','uint16',0xff); m.append('r','uint8',rgb[0]); m.append('g','uint8',rgb[1]); m.append('b','uint8',rgb[2]); m.append('repeat','uint8',0); m.append('t1','uint16',t[0]); m.append('t2','uint16',t[1]); m.append('comp_mask','uint16',0x3f)
    m.cmd_id = 0x32; emit(f"led_{name}", m)
for k, v in out.items(): print(k, v)
if "--json" in sys.argv:
    json.dump(out, open(sys.argv[sys.argv.index("--json") + 1], "w"), indent=1)
