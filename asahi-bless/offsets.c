#include <inttypes.h>
#include <stdio.h>
#include <stddef.h>

#define MAX_CKSUM_SIZE 8

typedef uint64_t oid_t;
typedef uint64_t xid_t;

typedef struct obj_phys {
    uint8_t o_cksum[MAX_CKSUM_SIZE];
    oid_t o_oid;
    xid_t o_xid;
    uint32_t o_type;
    uint32_t o_subtype;
} obj_phys_t;

typedef int64_t paddr_t;

typedef struct prange {
    paddr_t pr_start_paddr;
    uint64_t pr_block_count;
} prange_t;

typedef unsigned char uuid_t[16];

#define NX_MAX_FILE_SYSTEMS 100

typedef enum {
    NX_CNTR_OBJ_CKSUM_SET = 0,
    NX_CNTR_OBJ_CKSUM_FAIL = 1,
    NX_NUM_COUNTERS = 32
} nx_counter_id_t;

#define NX_EPH_INFO_COUNT 4

typedef struct nx_superblock {
    obj_phys_t nx_o;
    uint32_t nx_magic;
    uint32_t nx_block_size;
    uint64_t nx_block_count;
    uint64_t nx_features;
    uint64_t nx_readonly_compatible_features;
    uint64_t nx_incompatible_features;
    uuid_t nx_uuid;
    oid_t nx_next_oid;
    xid_t nx_next_xid;
    uint32_t nx_xp_desc_blocks;
    uint32_t nx_xp_data_blocks;
    paddr_t nx_xp_desc_base;
    paddr_t nx_xp_data_base;
    uint32_t nx_xp_desc_next;
    uint32_t nx_xp_data_next;
    uint32_t nx_xp_desc_index;
    uint32_t nx_xp_desc_len;
    uint32_t nx_xp_data_index;
    uint32_t nx_xp_data_len;
    oid_t nx_spaceman_oid;
    oid_t nx_omap_oid;
    oid_t nx_reaper_oid;
    uint32_t nx_test_type;
    uint32_t nx_max_file_systems;
    oid_t nx_fs_oid[NX_MAX_FILE_SYSTEMS];
    uint64_t nx_counters[NX_NUM_COUNTERS];
    prange_t nx_blocked_out_prange;
    oid_t nx_evict_mapping_tree_oid;
    uint64_t nx_flags;
    paddr_t nx_efi_jumpstart;
    uuid_t nx_fusion_uuid;
    prange_t nx_keylocker;
    uint64_t nx_ephemeral_info[NX_EPH_INFO_COUNT];
    oid_t nx_test_oid;
    oid_t nx_fusion_mt_oid;
    oid_t nx_fusion_wbc_oid;
    prange_t nx_fusion_wbc;
    uint64_t nx_newest_mounted_version;
    prange_t nx_mkb_locker;
} nx_superblock_t;

typedef struct omap_phys {
    obj_phys_t om_o;
    uint32_t om_flags;
    uint32_t om_snap_count;
    uint32_t om_tree_type;
    uint32_t om_snapshot_tree_type;
    oid_t om_tree_oid;
    oid_t om_snapshot_tree_oid;
    xid_t om_most_recent_snap;
    xid_t om_pending_revert_min;
    xid_t om_pending_revert_max;
} omap_phys_t;

typedef struct nloc {
    uint16_t off;
    uint16_t len;
}  nloc_t;

typedef struct btree_node_phys {
    obj_phys_t btn_o;
    uint16_t btn_flags;
    uint16_t btn_level;
    uint32_t btn_nkeys;
    nloc_t btn_table_space;
    nloc_t btn_free_space;
    nloc_t btn_key_free_list;
    nloc_t btn_val_free_list;
    uint64_t btn_data[];
} btree_node_phys_t;

typedef struct btree_info_fixed {
    uint32_t bt_flags;
    uint32_t bt_node_size;
    uint32_t bt_key_size;
    uint32_t bt_val_size;
}  btree_info_fixed_t;

typedef struct btree_info {
    btree_info_fixed_t bt_fixed;
    uint32_t bt_longest_key;
    uint32_t bt_longest_val;
    uint64_t bt_key_count;
    uint64_t bt_node_count;
} btree_info_t;

typedef uint32_t crypto_flags_t;
typedef uint32_t cp_key_class_t;
typedef uint32_t cp_key_os_version_t;
typedef uint16_t cp_key_revision_t;

struct wrapped_meta_crypto_state {
    uint16_t major_version;
    uint16_t minor_version;
    crypto_flags_t cpflags;
    cp_key_class_t persistent_class;
    cp_key_os_version_t key_os_version;
    cp_key_revision_t key_revision;
    uint16_t unused;
} __attribute__((aligned(2), packed));
typedef struct wrapped_meta_crypto_state wrapped_meta_crypto_state_t;

#define APFS_MODIFIED_NAMELEN 32

typedef struct apfs_modified_by {
    uint8_t id[APFS_MODIFIED_NAMELEN];
    uint64_t timestamp;
    xid_t last_xid;
} apfs_modified_by_t;

#define APFS_MAX_HIST 8
#define APFS_VOLNAME_LEN 256

typedef struct apfs_superblock {
    obj_phys_t apfs_o;
    uint32_t apfs_magic;
    uint32_t apfs_fs_index;
    uint64_t apfs_features;
    uint64_t apfs_readonly_compatible_features;
    uint64_t apfs_incompatible_features;
    uint64_t apfs_unmount_time;
    uint64_t apfs_fs_reserve_block_count;
    uint64_t apfs_fs_quota_block_count;
    uint64_t apfs_fs_alloc_count;
    wrapped_meta_crypto_state_t apfs_meta_crypto;
    uint32_t apfs_root_tree_type;
    uint32_t apfs_extentref_tree_type;
    uint32_t apfs_snap_meta_tree_type;
    oid_t apfs_omap_oid;
    oid_t apfs_root_tree_oid;
    oid_t apfs_extentref_tree_oid;
    oid_t apfs_snap_meta_tree_oid;
    xid_t apfs_revert_to_xid;
    oid_t apfs_revert_to_sblock_oid;
    uint64_t apfs_next_obj_id;
    uint64_t apfs_num_files;
    uint64_t apfs_num_directories;
    uint64_t apfs_num_symlinks;
    uint64_t apfs_num_other_fsobjects;
    uint64_t apfs_num_snapshots;
    uint64_t apfs_total_blocks_alloced;
    uint64_t apfs_total_blocks_freed;
    uuid_t apfs_vol_uuid;
    uint64_t apfs_last_mod_time;
    uint64_t apfs_fs_flags;
    apfs_modified_by_t apfs_formatted_by;
    apfs_modified_by_t apfs_modified_by[APFS_MAX_HIST];
    uint8_t apfs_volname[APFS_VOLNAME_LEN];
    uint32_t apfs_next_doc_id;
    uint16_t apfs_role;
    uint16_t reserved;
    xid_t apfs_root_to_xid;
    oid_t apfs_er_state_oid;
    uint64_t apfs_cloneinfo_id_epoch;
    uint64_t apfs_cloneinfo_xid;
    oid_t apfs_snap_meta_ext_oid;
    uuid_t apfs_volume_group_id;
    oid_t apfs_integrity_meta_oid;
    oid_t apfs_fext_tree_oid;
    uint32_t apfs_fext_tree_type;
    uint32_t reserved_type;
    oid_t reserved_oid;
} apfs_superblock_t;

int main() {
    printf("%ld\n", offsetof(apfs_superblock_t, apfs_role));
}
