import java.nio.file.Files;
import java.nio.file.Path;
import net.minecraft.SharedConstants;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.Block;

/** Run against the vanilla 26.3 server and libraries with the output path as the only argument. */
public class FeatureShapes {
    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        byte[] masks = new byte[Block.BLOCK_STATE_REGISTRY.size() * 2];
        var level = EmptyBlockGetter.INSTANCE;
        var pos = BlockPos.ZERO;
        for (var state : Block.BLOCK_STATE_REGISTRY) {
            int i = Block.getId(state) * 2;
            var support = state.getBlockSupportShape(level, pos);
            var collision = state.getCollisionShape(level, pos);
            if (state.isCollisionShapeFullBlock(level, pos)) masks[i] |= 64;
            if (state.canBeReplaced()) masks[i] |= (byte)128;
            if (state.isSolidRender()) masks[i + 1] |= (byte)128;
            for (var face : Direction.values()) {
                int bit = 1 << face.ordinal();
                if (Block.isFaceFull(support, face) || Block.isFaceFull(collision, face)) masks[i] |= bit;
                if (state.isFaceSturdy(level, pos, face)) masks[i + 1] |= bit;
            }
        }
        Files.write(Path.of(args[0]), masks);
    }
}
